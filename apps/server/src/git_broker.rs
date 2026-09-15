//! Local-only Git adapter. The SQL service owns authorization and serialization.
//! Platform storage and Git metadata must be outside the executor writable roots.
use crate::{
    workspace::{Manifest, Workspace},
    workspace_files::{self as files, Result, require},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CONFIG: &[u8] = b"[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n";

#[derive(Clone)]
pub struct GitBroker {
    root: PathBuf,
}

impl GitBroker {
    /// A fresh platform-owned canonical repository. Import only a local bundle;
    /// remote acquisition and credentials belong to the later delivery adapter.
    pub fn initialize(root: &Path, bundle: &Path) -> Result<Self> {
        initialize_directories(root)?;
        let broker = Self {
            root: root.to_owned(),
        };
        let output = base_command()
            .args(["init", "--bare", "--template="])
            .arg(broker.canonical())
            .output()?;
        require(output.status.success(), "canonical initialization failed")?;
        require(
            files::read(&broker.canonical().join("config"))? == CONFIG,
            "unexpected canonical config",
        )?;
        broker.import(bundle)?;
        Ok(broker)
    }

    pub fn open(root: &Path) -> Result<Self> {
        let broker = Self {
            root: root.to_owned(),
        };
        broker.check()?;
        Ok(broker)
    }

    fn canonical(&self) -> PathBuf {
        self.root.join("canonical.git")
    }

    fn check(&self) -> Result<()> {
        files::safe(&self.canonical())?;
        require(
            files::read(&self.canonical().join("config"))? == CONFIG,
            "untrusted repository config",
        )
    }

    pub fn path(&self, run: &str) -> Result<PathBuf> {
        files::component(run)?;
        Ok(self.root.join("runs").join(run))
    }

    pub fn archive_path(&self, run: &str) -> Result<PathBuf> {
        files::component(run)?;
        Ok(self.root.join("archives").join(run))
    }

    fn metadata(&self, workspace: &Workspace) -> Result<PathBuf> {
        files::component(&workspace.key.run_id)?;
        let metadata = self
            .canonical()
            .join("worktrees")
            .join(&workspace.key.run_id);
        files::safe(&metadata)?;
        require(
            !metadata.join("config.worktree").exists(),
            "worktree config rejected",
        )?;
        Ok(metadata)
    }

    fn owned(&self, workspace: &Workspace) -> Result<PathBuf> {
        self.check()?;
        let path = self.path(&workspace.key.run_id)?;
        require(
            path.to_str() == Some(&workspace.path),
            "workspace path mismatch",
        )?;
        let metadata = self.metadata(workspace)?;
        self.check_pointers(workspace, &path, &metadata)?;
        Ok(metadata)
    }

    fn check_pointers(&self, workspace: &Workspace, path: &Path, metadata: &Path) -> Result<()> {
        let pointer = format!("gitdir: {}\n", metadata.display());
        require(
            files::read(&path.join(".git"))? == pointer.as_bytes(),
            "Git ownership mismatch",
        )?;
        require(
            files::read(&metadata.join("gitdir"))?
                == format!("{}\n", path.join(".git").display()).as_bytes(),
            "reverse Git ownership mismatch",
        )?;
        require(
            files::read(&metadata.join("HEAD"))?
                == format!("ref: refs/heads/{}\n", workspace.branch).as_bytes(),
            "branch ownership mismatch",
        )?;
        Ok(())
    }

    fn git(&self, workspace: Option<&Workspace>, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        self.check()?;
        let mut command = base_command();
        match workspace {
            Some(workspace) => {
                command.arg("--git-dir").arg(self.owned(workspace)?);
                command.arg("--work-tree").arg(&workspace.path);
                command.current_dir(&workspace.path);
            }
            None => {
                command.arg("--git-dir").arg(self.canonical());
            }
        }
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("missing Git stdin")?
            .write_all(input)?;
        let output = child.wait_with_output()?;
        require(output.status.success(), "local Git operation failed")?;
        Ok(output.stdout)
    }

    fn oid(&self, workspace: Option<&Workspace>, args: &[&str], input: &[u8]) -> Result<String> {
        let value = String::from_utf8(self.git(workspace, args, input)?)?
            .trim()
            .to_owned();
        valid_oid(&value)?;
        Ok(value)
    }

    fn import(&self, bundle: &Path) -> Result<()> {
        files::safe(bundle)?;
        let path = bundle.to_str().ok_or("invalid bundle path")?;
        self.git(None, &["bundle", "verify", path], b"")?;
        self.git(
            None,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                path,
                "refs/*:refs/import/*",
            ],
            b"",
        )?;
        Ok(())
    }

    pub fn prepare(&self, workspace: &Workspace, checkout: bool) -> Result<()> {
        self.validate_new(workspace)?;
        let flag = if checkout {
            "--checkout"
        } else {
            "--no-checkout"
        };
        self.git(
            None,
            &[
                "worktree",
                "add",
                flag,
                "-b",
                &workspace.branch,
                &workspace.path,
                &workspace.baseline,
            ],
            b"",
        )?;
        self.owned(workspace)?;
        Ok(())
    }

    fn validate_new(&self, workspace: &Workspace) -> Result<()> {
        self.validate_location(workspace)?;
        valid_oid(&workspace.baseline)?;
        self.check_tree(&workspace.baseline)
    }

    fn validate_location(&self, workspace: &Workspace) -> Result<()> {
        files::safe(
            self.path(&workspace.key.run_id)?
                .parent()
                .ok_or("missing Run parent")?,
        )?;
        require(
            workspace.branch
                == format!("ai/req-{}-{}", workspace.requirement, workspace.key.run_id),
            "invalid managed branch",
        )?;
        require(
            self.path(&workspace.key.run_id)?.to_str() == Some(&workspace.path),
            "wrong workspace path",
        )?;
        require(
            !Path::new(&workspace.path).exists(),
            "Run path already used",
        )?;
        Ok(())
    }

    pub fn head(&self, workspace: &Workspace) -> Result<String> {
        self.oid(Some(workspace), &["rev-parse", "HEAD"], b"")
    }

    /// The platform stages source changes because executor Git metadata is
    /// read-only. Messages are stdin data, never options or shell.
    pub fn commit(&self, workspace: &Workspace, message: &str) -> Result<String> {
        require(
            !message.trim().is_empty() && message.len() <= 16384,
            "invalid commit message",
        )?;
        self.inspect(workspace)?;
        let head = self.head(workspace)?;
        let tree = self.staged_tree(workspace)?;
        let commit = self.oid(
            Some(workspace),
            &["commit-tree", &tree, "-p", &head],
            message.as_bytes(),
        )?;
        self.git(
            Some(workspace),
            &["update-ref", "HEAD", &commit, &head],
            b"",
        )?;
        Ok(commit)
    }

    fn stage(&self, workspace: &Workspace) -> Result<()> {
        self.git(
            Some(workspace),
            &[
                "add",
                "--all",
                "--",
                ".",
                ":(exclude)target",
                ":(exclude)node_modules",
                ":(exclude).angular",
            ],
            b"",
        )?;
        Ok(())
    }

    fn staged_tree(&self, workspace: &Workspace) -> Result<String> {
        self.stage(workspace)?;
        let tree = self.oid(Some(workspace), &["write-tree"], b"")?;
        self.check_tree(&tree)?;
        Ok(tree)
    }

    fn inspect(
        &self,
        workspace: &Workspace,
    ) -> Result<(Vec<crate::workspace::FileEntry>, Vec<String>)> {
        self.owned(workspace)?;
        let tracked = self.git(Some(workspace), &["ls-files", "-z"], b"")?;
        for name in tracked.split(zero).filter(nonempty) {
            let path = std::str::from_utf8(name)?;
            safe_content_path(path)?;
            let first = path.split('/').next().ok_or("invalid tracked path")?;
            require(
                !files::CACHE_ROOTS.contains(&first),
                "tracked cache would be excluded",
            )?;
        }
        files::inventory(Path::new(&workspace.path))
    }

    fn check_tree(&self, tree: &str) -> Result<()> {
        let listing = self.git(None, &["ls-tree", "-r", "-z", tree], b"")?;
        for entry in listing.split(zero).filter(nonempty) {
            let (metadata, path) = std::str::from_utf8(entry)?
                .split_once('\t')
                .ok_or("invalid tree entry")?;
            require(
                metadata.starts_with("100644 ") || metadata.starts_with("100755 "),
                "symlink or submodule tree rejected",
            )?;
            safe_content_path(path)?;
        }
        Ok(())
    }

    fn check_history(&self, head: &str, index_tree: &str) -> Result<()> {
        self.check_tree(index_tree)?;
        let commits = String::from_utf8(self.git(None, &["rev-list", head], b"")?)?;
        for commit in commits.lines() {
            self.check_tree(commit)?;
        }
        Ok(())
    }

    /// Called only after persisted descendant quiescence. A failed attempt owns
    /// its partial directory forever until explicit operator reconciliation.
    pub fn preserve(&self, workspace: &Workspace) -> Result<Manifest> {
        let directory = self.begin_archive(&workspace.key.run_id)?;
        let (entries, excluded) = self.inspect(workspace)?;
        let mut manifest = self.save_history(workspace, &directory)?;
        manifest.files = entries;
        manifest.excluded = excluded;
        let storage = directory.join("files");
        fs::create_dir(&storage)?;
        files::copy_files(Path::new(&workspace.path), &storage, &manifest.files)?;
        self.seal(&directory, &manifest)?;
        Ok(manifest)
    }

    fn begin_archive(&self, run: &str) -> Result<PathBuf> {
        let directory = self.archive_path(run)?;
        let parent = directory.parent().ok_or("missing archive parent")?;
        files::safe(parent)?;
        fs::create_dir(&directory)?;
        files::sync_path(parent)?;
        files::write(
            &directory.join("pending"),
            b"files are not a database reference\n",
            false,
        )?;
        Ok(directory)
    }

    fn save_history(&self, workspace: &Workspace, directory: &Path) -> Result<Manifest> {
        let head = self.head(workspace)?;
        let index_tree = self.oid(Some(workspace), &["write-tree"], b"")?;
        self.check_history(&head, &index_tree)?;
        let bundle_digest = self.save_bundle(workspace, directory, &head, &index_tree)?;
        let index_digest = self.save_index(workspace, directory)?;
        Ok(Manifest {
            workspace: workspace.clone(),
            head,
            index_tree,
            files: Vec::new(),
            excluded: Vec::new(),
            bundle_digest,
            index_digest,
        })
    }

    fn save_bundle(
        &self,
        workspace: &Workspace,
        directory: &Path,
        head: &str,
        tree: &str,
    ) -> Result<String> {
        let commit = self.oid(
            Some(workspace),
            &["commit-tree", tree, "-p", head],
            b"preserved index\n",
        )?;
        let anchor = format!("refs/preservation/{}/index", workspace.key.run_id);
        self.git(
            None,
            &[
                "update-ref",
                &anchor,
                &commit,
                "0000000000000000000000000000000000000000",
            ],
            b"",
        )?;
        let bundle = directory.join("history.bundle");
        self.git(
            Some(workspace),
            &["bundle", "create", path_text(&bundle)?, "HEAD", &anchor],
            b"",
        )?;
        self.seal_bundle(&bundle)
    }

    fn seal_bundle(&self, bundle: &Path) -> Result<String> {
        files::sync_path(bundle)?;
        self.verify_bundle(bundle)?;
        files::digest_file(bundle)
    }

    fn verify_bundle(&self, bundle: &Path) -> Result<()> {
        self.git(None, &["bundle", "verify", path_text(bundle)?], b"")?;
        Ok(())
    }

    fn save_index(&self, workspace: &Workspace, directory: &Path) -> Result<String> {
        let index = files::read(&self.metadata(workspace)?.join("index"))?;
        files::write(&directory.join("index"), &index, false)?;
        files::digest(&index)
    }

    fn seal(&self, directory: &Path, manifest: &Manifest) -> Result<()> {
        self.unchanged(manifest)?;
        files::write(
            &directory.join("manifest.json"),
            &serde_json::to_vec(manifest)?,
            false,
        )?;
        self.verify(manifest)
    }

    fn unchanged(&self, manifest: &Manifest) -> Result<()> {
        let workspace = &manifest.workspace;
        require(
            self.inspect(workspace)? == (manifest.files.clone(), manifest.excluded.clone()),
            "source changed during preservation",
        )?;
        require(
            self.head(workspace)? == manifest.head,
            "HEAD changed during preservation",
        )?;
        verify_digest(
            &self.metadata(workspace)?.join("index"),
            &manifest.index_digest,
        )
    }

    pub fn verify(&self, manifest: &Manifest) -> Result<()> {
        let directory = self.archive_path(&manifest.workspace.key.run_id)?;
        require(
            files::read(&directory.join("manifest.json"))? == serde_json::to_vec(manifest)?,
            "manifest identity mismatch",
        )?;
        verify_digest(&directory.join("history.bundle"), &manifest.bundle_digest)?;
        self.verify_bundle(&directory.join("history.bundle"))?;
        verify_digest(&directory.join("index"), &manifest.index_digest)?;
        require(
            files::inventory(&directory.join("files"))?.0 == manifest.files,
            "archived files mismatch",
        )
    }

    pub fn restore(&self, target: &Workspace, manifest: &Manifest) -> Result<()> {
        let archive = self.prepare_restore(target, manifest, false)?;
        self.restore_files(target, manifest, &archive)?;
        self.verify_restored(target, manifest)
    }

    fn restore_files(&self, target: &Workspace, manifest: &Manifest, archive: &Path) -> Result<()> {
        files::copy_files(
            &archive.join("files"),
            Path::new(&target.path),
            &manifest.files,
        )?;
        files::write(
            &self.metadata(target)?.join("index"),
            &files::read(&archive.join("index"))?,
            false,
        )?;
        Ok(())
    }

    fn verify_restored(&self, target: &Workspace, manifest: &Manifest) -> Result<()> {
        require(
            self.head(target)? == manifest.head,
            "restored HEAD mismatch",
        )?;
        require(
            self.oid(Some(target), &["write-tree"], b"")? == manifest.index_tree,
            "restored index mismatch",
        )?;
        require(
            self.inspect(target)?.0 == manifest.files,
            "restored worktree mismatch",
        )
    }

    fn prepare_restore(
        &self,
        target: &Workspace,
        manifest: &Manifest,
        checkout: bool,
    ) -> Result<PathBuf> {
        self.verify(manifest)?;
        require(
            target.key.run_id != manifest.workspace.key.run_id,
            "recovery requires a new Run",
        )?;
        require(
            target.baseline == manifest.head,
            "recovery baseline must be preserved HEAD",
        )?;
        let archive = self.archive_path(&manifest.workspace.key.run_id)?;
        self.import(&archive.join("history.bundle"))?;
        self.prepare(target, checkout)?;
        Ok(archive)
    }

    pub fn restore_candidate(&self, target: &Workspace, manifest: &Manifest) -> Result<()> {
        self.prepare_restore(target, manifest, true)?;
        require(
            self.head(target)? == manifest.head,
            "candidate HEAD mismatch",
        )?;
        self.git(
            Some(target),
            &[
                "diff",
                "--exit-code",
                "--no-ext-diff",
                "--no-textconv",
                "HEAD",
                "--",
            ],
            b"",
        )?;
        Ok(())
    }
}

fn valid_oid(value: &str) -> Result<()> {
    require(
        value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid Git SHA",
    )
}

fn safe_content_path(path: &str) -> Result<()> {
    for part in path.split('/') {
        require(
            !files::forbidden(part),
            "credential or Git configuration path rejected",
        )?;
    }
    Ok(())
}

fn zero(byte: &u8) -> bool {
    *byte == 0
}
fn nonempty(bytes: &&[u8]) -> bool {
    !bytes.is_empty()
}

fn base_command() -> Command {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "CodexSymphony")
        .env("GIT_AUTHOR_EMAIL", "broker@localhost")
        .env("GIT_COMMITTER_NAME", "CodexSymphony")
        .env("GIT_COMMITTER_EMAIL", "broker@localhost")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "credential.helper=",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.file.allow=always",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.fsync=all",
        ]);
    command
}

fn initialize_directories(root: &Path) -> Result<()> {
    files::safe(root.parent().ok_or("missing root parent")?)?;
    fs::create_dir(root)?;
    for name in ["runs", "archives"] {
        fs::create_dir(root.join(name))?;
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str> {
    Ok(path.to_str().ok_or("invalid bundle path")?)
}

fn verify_digest(path: &Path, expected: &str) -> Result<()> {
    require(
        files::digest(&files::read(path)?)? == expected,
        "content digest mismatch",
    )
}
