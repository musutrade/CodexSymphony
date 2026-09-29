"""Verify-only admission of a trusted complete local Gate PASS for one Actions attempt.

Every event (pull_request, main push, workflow_dispatch and reruns) only verifies:
nothing here installs dependencies, starts a database, captures or runs a Gate.
Missing, mismatched, blocked or unavailable evidence fails this attempt; it is
never written to the ledger, so the same PASS verifies again once its evidence or
transport is restored. A rejection never starts a Gate. Its category separates a
quality or evidence rejection from expired retention (retention-expired) and from a
GitHub transport failure (transport), and its text names the matching recovery.

The admitted PASS is looked up by the exact remote tree and the installed
approval, then re-verified by the ledger (later FAIL, revocation or unconcluded
attempt blocks it). Its executing host's environment is the fingerprint the Gate
retained, not the Actions runner. Every pull request proof, and a main commit
other than the validated one, is only admitted under the installed equivalence
audit of exactly this candidate: a pull request lands on main as a squash or merge
commit verified from its proof, so an exact PR head needs the audit too; only the
validated commit itself on main is admitted as exact without one. One document
binds one approval, tree, validation, executing environment (fingerprint and
retained environment evidence) and the approved runtime tools, and classifies every
reviewed Git metadata read (label-only, tree-file-selection, fixed-approved-input or
not-executed) with references to the approved files, baseline values and retained
evidence it relies on. Admission only checks that those references exist with their
approved values and that each stated runtime chain line contains its stated text;
that is not a proof that a classification is right or a branch unreachable, which
remains the reviewer's independent audit. There are no lists or wildcards: each
candidate needs its own reviewed audit, and a missing match is rejected. Commit
parents are an audit label, never an equivalence condition.

Targets: an attempt is classified by what it actually names. A pull_request
attempt, or a dispatch of the head of exactly one open pull request, follows the PR
rule (head contains the latest main). A push, or a dispatch of a commit on main,
follows the main rule: the validated commit itself, or the merge commit M of exactly
one merged pull request whose confirmed PR publication binds the same repository,
PR, head H, base B, tree and validation, with M's parents [B] (squash) or [B, H]
(merge). Anything else is rejected; a dispatch cannot bypass either rule.

Publication: a retention pin protects the validation's evidence and carries the
unconfirmed publication before success is published; the pin lock stays held across
the success PATCH. Only a success whose check run reads back as this attempt's
success, while its pin is still live, is confirmed in place; announced, failed,
cancelled or interrupted attempts are never merge sources. A merged PR proof is
extended to cover its main verification (sweep); publications exist only in live
pins, so they are charged to and bounded by the records budget.

Lock order: ledger shared lock -> pin lock; the pin lock holder waits for no lock.
The ledger's shared lock is held from verification until the App check is
published, and a persisted head anchor detects a rolled-back ledger.
"""
import calendar
import hashlib
import http.client
import importlib.util
import json
from pathlib import Path
import re
import sys
import time

# Installed releases carry evidence_ledger.py beside this module; the repository copy lives in tools/.
HERE = Path(__file__).resolve().parent
sys.path.append(str(HERE if (HERE / 'evidence_ledger.py').is_file() else HERE.parent))
import evidence_ledger as ledger  # noqa: E402
from github import installation_token, request


def load_pins():
    """Installed releases carry evidence_pins.py beside this module; the repository copy lives in tools/quality-host."""
    path = HERE / 'evidence_pins.py'
    path = path if path.is_file() else HERE.parent / 'quality-host/evidence_pins.py'
    spec = importlib.util.spec_from_file_location('evidence_pins', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pins = load_pins()

SCHEMA = 'codexsymphony-ci-publication/v1'
EXACT = 'exact-commit/v1'
# The published commit differs from the validated one only in commit metadata.
TREE_EQUIVALENCE = 'tree-equivalence/v1'
RULES = (EXACT, TREE_EQUIVALENCE)
AUDIT_SCHEMA = 'codexsymphony-equivalence-audit/v3'
# Enforced here; an audit document must state exactly these to be accepted.
CONDITIONS = ('same-tree', 'same-approval', 'same-executing-environment',
              'git-metadata-reads-classified-and-bound')
# One candidate per document: every binding is a scalar compared with the admitted record.
BINDINGS = ('approval', 'tree', 'validation_id', 'environment', 'environment_evidence_sha256')
AUDIT_KEYS = ('schema', 'rule', 'conditions', 'runtime_files', 'git_metadata_reads', *BINDINGS)
# The reviewer's classification of each read, by the approved file performing it. The fields each
# classification must add are only references checked against the approval and the record; the
# classification itself is the reviewer's judgement, never derived here.
READ_KEYS = ('path', 'reader', 'read', 'use', 'verdict')
LABEL_ONLY = 'label-only'
TREE_FILE_SELECTION = 'tree-file-selection'
FIXED_APPROVED_INPUT = 'fixed-approved-input'
NOT_EXECUTED = 'not-executed'
VERDICTS = {LABEL_ONLY: (), FIXED_APPROVED_INPUT: ('inputs',), TREE_FILE_SELECTION: ('guards', 'retained'),
            NOT_EXECUTED: ('chain',)}
REFERENCE_KEYS = ('source', 'name', 'digest')
# Readers and guards are executed files; a chain may also cite the approved configuration.
READERS = ('runtime_files', 'trusted_files')
CHAIN_SOURCES = ('runtime_files', 'config_files', 'trusted_files')
CHAIN_KEYS = ('source', 'name', 'digest', 'line', 'text')
INPUT_KEYS = ('source', 'name', 'value')
# A file selection is bound to the tree only through the host's retained source snapshot and archive.
SELECTION_EVIDENCE = ('source-inputs.json', 'source-archive.json')
BASELINE_INPUTS = (('baseline', 'sha256'), ('baseline', 'commit'), ('baseline', 'path'))
OBJECT = re.compile('[0-9a-f]{40}')
RETENTION_EXPIRED = 'retention-expired'
TRANSPORT = 'transport'


class Rejected(ValueError):
    """Evidence does not admit this attempt; category names the recovery."""

    def __init__(self, message, category='rejected'):
        super().__init__(message)
        self.category = category


def api(path, token, method='GET', body=None):
    """A GitHub read or write; a transport failure judges no evidence."""
    try:
        return request(path, token, method, body)
    except (OSError, http.client.HTTPException) as error:
        raise Rejected(type(error).__name__ + ': ' + str(error), TRANSPORT) from None


def token_of(config):
    try:
        return installation_token(config)
    except (OSError, http.client.HTTPException) as error:
        raise Rejected(type(error).__name__ + ': ' + str(error), TRANSPORT) from None


def approval_identity(approval):
    """Identical to environment_contract.digest, which names the approval in ledger inputs."""
    return hashlib.sha256(json.dumps(approval, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def object_id(value, what):
    if not isinstance(value, str) or not OBJECT.fullmatch(value):
        raise Rejected('GitHub returned an invalid ' + what)
    return value


def remote_commit(prefix, token, sha):
    """(tree, parents) of the exact remote commit."""
    commit = api(prefix + '/git/commits/' + sha, token)
    if commit.get('sha') != sha:
        raise Rejected('GitHub returned another commit')
    parents = [object_id(parent.get('sha'), 'parent') for parent in commit.get('parents', [])]
    return object_id(commit.get('tree', {}).get('sha'), 'tree'), parents


def main_head(prefix, token):
    ref = api(prefix + '/git/ref/heads/main', token)
    if ref.get('object', {}).get('type') != 'commit':
        raise Rejected('main does not name a commit')
    return object_id(ref['object'].get('sha'), 'main commit')


def contains(prefix, token, base, head):
    """True when head contains base: the comparison base...head is not behind."""
    comparison = api(prefix + '/compare/' + base + '...' + head, token)
    return comparison.get('behind_by') == 0 and comparison.get('status') in ('ahead', 'identical')


def associated(prefix, token, sha):
    pulls = api(prefix + '/commits/' + sha + '/pulls', token)
    if not isinstance(pulls, list):
        raise Rejected('GitHub returned no pull request list')
    return pulls


def dispatched(run, prefix, token):
    """A dispatch follows the rule of what it names: a main commit, or one open PR head."""
    if contains(prefix, token, run['head_sha'], main_head(prefix, token)):
        return None
    heads = [pr['number'] for pr in associated(prefix, token, run['head_sha'])
             if pr.get('state') == 'open' and pr.get('head', {}).get('sha') == run['head_sha']]
    if len(heads) != 1:
        raise Rejected('dispatch names neither a main commit nor the head of exactly one open pull request')
    return heads[0]


def target(run, prefix, token):
    """The PR number the attempt binds, or None for a main commit."""
    if run['event'] == 'push':
        return None
    if run['event'] == 'workflow_dispatch':
        return dispatched(run, prefix, token)
    listed = run.get('pull_requests') or []
    if len(listed) != 1:
        raise Rejected('attempt must belong to exactly one pull request')
    return int(listed[0]['number'])


def open_pull_request(prefix, token, number, sha):
    pr = api(prefix + '/pulls/' + str(number), token)
    if pr.get('state') != 'open' or pr['head']['sha'] != sha or pr['base']['ref'] != 'main':
        raise Rejected('pull request head or base changed')
    return pr['number']


def candidate(run, number, prefix, token):
    """Base and PR the attempt binds; a main commit must lie on main, a PR head must contain main."""
    base = main_head(prefix, token)
    on_main = number is None
    if not on_main:
        number = open_pull_request(prefix, token, number, run['head_sha'])
    older, newer = (run['head_sha'], base) if on_main else (base, run['head_sha'])
    if not contains(prefix, token, older, newer):
        raise Rejected('commit is not on main' if on_main else 'candidate does not contain the latest main')
    return {'base': base, 'pull_request': number}


def check_runtime(approval):
    """A past verdict never excuses a changed installed host runtime."""
    for name, digest in approval['runtime_files'].items():
        path = Path(name)
        if path.resolve() != path or ledger.file_digest(path) != digest:
            raise Rejected('approved runtime changed: ' + name)


def select(root, approval, tree, observed):
    """Newest PASS for this approval and tree, re-verified; a blocked newest PASS is not skipped."""
    events, _ = ledger.read_events(ledger.canonical_root(root))
    for event in reversed(events):
        if event['kind'] != 'pass':
            continue
        inputs = ledger.load_record(root, event['validation_id'])['inputs']
        if (inputs['approval'], inputs['tree']) == (approval, tree):
            return ledger.verify(root, event['validation_id'], inputs, observed)
    raise ledger.fail('missing', 'no complete local PASS for this tree and approval')


def check_record(record):
    """The executing host's retained environment and complete report back this PASS."""
    evidence = record['evidence']
    commit = record['inputs']['commit']
    environment = json.loads(Path(evidence['environment.json']['path']).read_text())
    if environment.get('fingerprint') != record['inputs']['environment']:
        raise Rejected('retained environment differs from the validated environment')
    report = json.loads(Path(evidence['report']['path']).read_text())
    if report.get('passed') is not True or report.get('evidence_complete') is not True:
        raise Rejected('validated report is incomplete')
    if report.get('source_identity') != 'working-tree:' + commit:
        raise Rejected('validated report names another commit')


def audit_file(rule):
    """The installed audit document, unchanged since installation."""
    if not isinstance(rule, dict) or rule.get('rule') != TREE_EQUIVALENCE:
        raise Rejected('no reviewed equivalence rule is installed; a pull request or a commit other than '
                       'the validated one needs the audit of this candidate')
    path = Path(rule['audit'])
    if path.resolve() != path or ledger.file_digest(path) != rule['audit_sha256']:
        raise Rejected('installed equivalence audit changed')
    audit = json.loads(path.read_text())
    if not isinstance(audit, dict) or sorted(audit) != sorted(AUDIT_KEYS):
        raise Rejected('equivalence audit fields differ from the reviewed schema')
    return audit


def malformed():
    return Rejected('equivalence audit has a malformed Git metadata read')


def unbound(path):
    return Rejected('Git metadata read binding does not match: ' + path)


def shaped(item, keys, texts):
    """item is a dict of exactly keys whose texts fields are non-empty strings."""
    return isinstance(item, dict) and sorted(item) == sorted(keys) and \
        all(isinstance(item[key], str) and item[key] for key in texts)


def items(value):
    """A non-empty list; the caller checks each entry."""
    if not isinstance(value, list) or not value:
        raise malformed()
    return value


def approved(reference, sources, approval, path):
    """A reference to a file this approval pins, from one of sources, with exactly the stated digest."""
    table = approval.get(reference['source']) if reference['source'] in sources else None
    if not isinstance(table, dict) or table.get(reference['name']) != reference['digest']:
        raise unbound(path)


def check_reader(reference, approval, path):
    """An approved executed file: the reader of a read, or a guard of its file selection."""
    if not shaped(reference, REFERENCE_KEYS, REFERENCE_KEYS):
        raise malformed()
    approved(reference, READERS, approval, path)


def labelled(entry, approval):
    """A label-only read binds nothing beyond its reader."""
    return None


def check_inputs(entry, approval):
    """Each approved baseline value the read actually uses, equal to the approval's value."""
    baseline = approval.get('baseline') if isinstance(approval.get('baseline'), dict) else {}
    for item in items(entry['inputs']):
        if not shaped(item, INPUT_KEYS, INPUT_KEYS):
            raise malformed()
        if (item['source'], item['name']) not in BASELINE_INPUTS or baseline.get(item['name']) != item['value']:
            raise unbound(entry['path'])


def check_selection(entry, approval):
    """Approved guards of the selected file set and the evidence names they retain (checked in check_binding)."""
    for guard in items(entry['guards']):
        check_reader(guard, approval, entry['path'])
    if not all(isinstance(name, str) and name for name in items(entry['retained'])):
        raise malformed()
    if not set(SELECTION_EVIDENCE) <= set(entry['retained']):
        raise Rejected('tree file selection must retain ' + ' and '.join(SELECTION_EVIDENCE) + ': ' + entry['path'])


def check_line(link, path):
    """The approved runtime file, re-read, has the stated text on the stated line.

    This only fixes where the reviewer looked; a line containing text is no proof that
    a branch is unreachable."""
    file = Path(link['name'])
    if file.resolve() != file:
        raise unbound(path)
    data = file.read_bytes()
    if hashlib.sha256(data).hexdigest() != link['digest']:
        raise unbound(path)
    try:
        lines = data.decode().splitlines()
    except UnicodeDecodeError:
        raise Rejected('Git metadata read chain cites a file that is not text: ' + path) from None
    if link['line'] > len(lines):
        raise Rejected('Git metadata read chain line is out of range: ' + path)
    if link['text'] not in lines[link['line'] - 1]:
        raise Rejected('Git metadata read chain line does not contain its text: ' + path)


def check_chain(entry, approval):
    """The ordered cited lines from the entry point to the reviewed unexecuted branch.

    Configuration and trusted files are checked by digest only: admission has no fixed
    readable copy of them."""
    for link in items(entry['chain']):
        if not shaped(link, CHAIN_KEYS, ('source', 'name', 'digest', 'text')) or \
                type(link['line']) is not int or link['line'] < 1:
            raise malformed()
        approved(link, CHAIN_SOURCES, approval, entry['path'])
        if link['source'] == 'runtime_files':
            check_line(link, entry['path'])


CHECKS = {LABEL_ONLY: labelled, FIXED_APPROVED_INPUT: check_inputs, TREE_FILE_SELECTION: check_selection,
          NOT_EXECUTED: check_chain}


def classification(entry):
    """The stated classification of a read that names its location."""
    if not isinstance(entry, dict) or not isinstance(entry.get('path'), str) or not entry['path'] or \
            not isinstance(entry.get('verdict'), str):
        raise malformed()
    if entry['verdict'] not in VERDICTS:
        raise Rejected('unknown Git metadata read classification: ' + entry['path'])
    return entry['verdict']


def check_read(entry, approval):
    """One reviewed read: location, approved reader, command, use, classification and exactly its references."""
    verdict = classification(entry)
    if not shaped(entry, READ_KEYS + VERDICTS[verdict], ('path', 'read', 'use')):
        raise malformed()
    check_reader(entry['reader'], approval, entry['path'])
    CHECKS[verdict](entry, approval)


def check_reads(reads, approval):
    """Each Git metadata read carries the reviewer's classification; the list itself proves nothing."""
    if not isinstance(reads, list) or not reads:
        raise Rejected('equivalence audit records no reviewed Git metadata reads')
    for entry in reads:
        check_read(entry, approval)


def audit_document(rule, approval):
    """The installed audit, structurally valid for this approval; installation and admission share it."""
    audit = audit_file(rule)
    if (audit['schema'], audit['rule'], audit['conditions']) != (AUDIT_SCHEMA, TREE_EQUIVALENCE, list(CONDITIONS)):
        raise Rejected('equivalence audit does not state the enforced conditions')
    if audit['approval'] != approval_identity(approval) or audit['runtime_files'] != approval['runtime_files']:
        raise Rejected('equivalence audit was not reviewed for this approval and its runtime tools')
    check_reads(audit['git_metadata_reads'], approval)
    return audit


def check_retained(audit, record):
    """Every evidence name a file selection relies on is retained by exactly this record."""
    for entry in audit['git_metadata_reads']:
        for name in entry.get('retained', ()):
            if name not in record['evidence']:
                raise Rejected('equivalence audit relies on evidence this record lacks: ' + name)


def check_binding(audit, record, tree):
    """The audit names exactly this record: its approval, tree, validation, executing environment and evidence."""
    actual = {'approval': record['inputs']['approval'], 'tree': tree, 'validation_id': record['validation_id'],
              'environment': record['inputs']['environment'],
              'environment_evidence_sha256': record['evidence']['environment.json']['sha256']}
    for name in BINDINGS:
        if audit[name] != actual[name]:
            raise Rejected('equivalence audit binds another ' + name)
    check_retained(audit, record)


def audited(config, approval, record, tree):
    """The installed audit's identity, only if it binds exactly this candidate."""
    rule = config.get('equivalence')
    check_binding(audit_document(rule, approval), record, tree)
    return rule['audit_sha256']


def subject(repository, number, head, base, tree):
    return {'repository': repository, 'pull_request': number, 'head': head, 'base': base, 'tree': tree}


def confirmed(config, bound, validation_id, sha):
    """A live confirmed PR publication of exactly this subject and validation, found by content, never by time."""
    try:
        found = pins.find(config['pins'], bound)
    except pins.PinError as error:
        raise ledger.fail(error.category, str(error)) from None
    if not found:
        raise missing_source(config, bound, validation_id, sha)
    matching = [value for value in found if value['validation_id'] == validation_id]
    if not matching:
        raise Rejected('merge source names another validation of this tree')
    return matching[0]


def stored_pins(config):
    """Every stored pin, expired ones included until the next pin write drops them, and the time they are judged at."""
    with pins.locked(config['pins']) as root:
        return list(pins.load(root)['pins'].values()), pins.now_ms()


def lapsed(value, bound, validation_id, moment):
    """A confirmed proof of exactly the required subject and validation whose retention has ended."""
    return (value['subject'], value['validation_id'], value['publication']['status']) == \
        (bound, validation_id, 'confirmed') and value['expires_at_ms'] <= moment


def admitted_merge(value, sha, validation_id, moment):
    """A live main publication of this merge, admitted earlier from the required PR proof."""
    publication = value['publication']
    return (value['subject']['pull_request'], value['subject']['head'], value['validation_id'], publication['status']) == \
        (None, sha, validation_id, 'confirmed') and 'merge_source' in publication and value['expires_at_ms'] > moment


def of_pull_request(value, bound, moment):
    """A live pin of the required pull request, of any head, base or tree."""
    return (value['subject']['repository'], value['subject']['pull_request']) == \
        (bound['repository'], bound['pull_request']) and value['expires_at_ms'] > moment


def missing_source(config, bound, validation_id, sha):
    """Why the required PR proof is absent, judged only by facts about that proof.

    Expired retention needs a retained fact that exactly this proof existed: its own
    stored, lapsed pin, or a main publication of this merge admitted from it. A live
    pin of the same pull request with another subject is a mismatch; without any fact
    the absence is not called expired."""
    stored, moment = stored_pins(config)
    if any(lapsed(value, bound, validation_id, moment) or admitted_merge(value, sha, validation_id, moment)
           for value in stored):
        return Rejected('the confirmed pull request publication of this merge expired or was released', RETENTION_EXPIRED)
    same = [value for value in stored if of_pull_request(value, bound, moment)]
    return unmatched(same, bound)


def unmatched(same, bound):
    """Rejection for an absent proof given the live pins of the required pull request."""
    if any(value['subject'] == bound for value in same):
        return Rejected('the pull request publication of this merge was never confirmed')
    if same:
        return Rejected('merge does not match the publication of its pull request')
    return Rejected('no live confirmed pull request publication for this merge; '
                    'no retained fact shows whether it expired or was never confirmed', 'missing')


def merged_pull_request(prefix, token, sha):
    """Read back the unique merged PR whose merge commit is sha."""
    merged = [pr['number'] for pr in associated(prefix, token, sha) if pr.get('merge_commit_sha') == sha]
    if len(merged) != 1:
        raise Rejected('main commit is not the merge commit of exactly one pull request')
    pr = api(prefix + '/pulls/' + str(merged[0]), token)
    if (pr.get('merged'), pr.get('base', {}).get('ref'), pr.get('merge_commit_sha')) != (True, 'main', sha):
        raise Rejected('pull request was not merged into main as this commit')
    return pr['number'], object_id(pr.get('head', {}).get('sha'), 'pull request head')


def merge_source(config, prefix, token, record, commit):
    """The confirmed PR publication a squash or merge commit on main came from."""
    sha, tree, parents = commit
    number, head = merged_pull_request(prefix, token, sha)
    if not parents or parents not in ([parents[0]], [parents[0], head]):
        raise Rejected('main commit is neither a squash nor a merge of the pull request head onto its base')
    return confirmed(config, subject(config['repository'], number, head, parents[0], tree), record['validation_id'], sha)


def anchor(home):
    """Last ledger head this host verified; the index must still extend it."""
    path = Path(home) / 'ledger-anchor.json'
    return json.loads(path.read_text()) if path.exists() else None


def advance(home, head):
    path = Path(home) / 'ledger-anchor.json'
    pending = path.with_suffix('.new')
    pending.write_text(json.dumps(head) + '\n')
    pending.replace(path)


def verify(run, config, home, identity, token, prefix, number):
    """The unpublished result for this attempt, or Rejected; nothing is recorded here."""
    approval = json.loads(Path(config['gate_approval']).read_text())
    if approval.get('execution_version') != 3:
        raise Rejected('verify-only admission requires the v3 host')
    check_runtime(approval)
    tree, parents = remote_commit(prefix, token, run['head_sha'])
    bound = candidate(run, number, prefix, token)
    record = select(Path(config['publication_ledger']), approval_identity(approval), tree, anchor(home))
    check_record(record)
    if number is not None:
        rule = pull_request_rule(config, approval, record, tree, run['head_sha'])
    elif record['inputs']['commit'] == run['head_sha']:
        rule = {'rule': EXACT}
    else:
        rule = main_equivalence(config, prefix, token, record, approval, (run['head_sha'], tree, parents))
    value = {'schema': SCHEMA, 'repository': config['repository'], 'sha': run['head_sha'],
             'tree': tree, 'parents': parents, 'event': run['event'], 'actions_attempt': identity, **bound, **rule,
             'validation_id': record['validation_id'], 'validated_commit': record['inputs']['commit'],
             'ledger_head': record['head'], 'report_sha256': record['evidence']['report']['sha256'],
             'full_suite_executed': False, 'created_at_ms': int(time.time() * 1000)}
    return value, record


def pull_request_rule(config, approval, record, tree, sha):
    """Every PR proof, exact or not, is admitted under its candidate's audit: main later verifies its merge from it."""
    audit = audited(config, approval, record, tree)
    return {'rule': EXACT if record['inputs']['commit'] == sha else TREE_EQUIVALENCE, 'audit_sha256': audit}


def main_equivalence(config, prefix, token, record, approval, commit):
    """A main commit other than the validated one is admitted only from its live confirmed PR publication."""
    audit = audited(config, approval, record, commit[1])
    source = merge_source(config, prefix, token, record, commit)
    return {'rule': TREE_EQUIVALENCE, 'audit_sha256': audit, 'merge_source': source['publication_id']}


def summary(result):
    return (f"Commit: `{result['sha']}`; Actions attempt: `{result['actions_attempt']}`\n\n"
            f"CI did not run tests or a Gate. Trusted complete local validation `{result['validation_id']}` "
            f"of commit `{result['validated_commit']}` admitted under `{result['rule']}` for tree `{result['tree']}`.\n\n"
            f"Base: `{result['base']}`. Report SHA-256: `{result['report_sha256']}`\n\n"
            f"Retention pin: `{result['pin_id']}`")


def complete(prefix, token, check_id, conclusion, title, text):
    api(prefix + '/check-runs/' + str(check_id), token, 'PATCH',
            {'status': 'completed', 'conclusion': conclusion,
             'completed_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
             'output': {'title': title, 'summary': text}})


def read_back(prefix, token, check_id, sha, identity):
    """Success is confirmed only when the check run reads back as this attempt's success."""
    check = api(prefix + '/check-runs/' + str(check_id), token)
    if (check.get('head_sha'), check.get('external_id'), check.get('status'), check.get('conclusion')) != \
            (sha, identity, 'completed', 'success'):
        raise Rejected('published success did not read back for this attempt')


def reserve(config, record, result, identity):
    """Pin the evidence and the unconfirmed publication for this attempt's retention window before success."""
    ttl = config['pin_ttl_seconds'] if result['pull_request'] is not None else config['audit_window_seconds']
    bound = subject(config['repository'], result['pull_request'], result['sha'], result['base'], result['tree'])
    try:
        budget = pins.records_budget(config['storage_deployment'])
        return pins.pin(config['pins'], budget, record, bound, identity, ttl, result)
    except pins.PinError as error:
        raise Rejected('retention not reserved: ' + str(error)) from None


def succeed(prefix, token, check_id, config, result, value):
    """PATCH success and confirm it while the pin lock keeps the evidence from cleanup."""
    try:
        with pins.holding(config['pins'], value['pin_id'], result['validation_id']) as root:
            complete(prefix, token, check_id, 'success', 'Trusted local complete validation verified', summary(result))
            read_back(prefix, token, check_id, result['sha'], result['actions_attempt'])
            # Re-read under the held lock across the expiry boundary: success outlives its pin only as a rejection.
            value = pins.confirm(root, value['pin_id'], result['validation_id'])
    except pins.PinError as error:
        raise expired(error) from None
    return result | {'status': 'confirmed', 'publication_id': value['publication_id'],
                     'pin_expires_at_ms': value['expires_at_ms']}


def expired(error):
    """A pin that lapsed is expired retention, not a quality verdict; other pin faults keep their category."""
    return Rejected(str(error), RETENTION_EXPIRED) if error.category == 'expired' else error


# Title and recovery per rejection category; every rejection states that no Gate ran or will start.
FAILURES = {
    RETENTION_EXPIRED: ('Evidence retention expired; no Gate started',
                        'This is not a quality failure: no validation result was contradicted; the retention that '
                        'proves this publication expired or was released. Recovery: first check whether the local '
                        'PASS still stands with its evidence retained (ledger verify of the validation, pin state). '
                        'For a pull request attempt whose PASS still stands, re-run the Actions attempt: it reserves '
                        'new retention and needs no new validation. For a main commit whose pull request proof '
                        'expired, or a PASS that no longer stands, start a new complete local validation of this '
                        'exact commit manually, then re-run the attempt.'),
    TRANSPORT: ('GitHub transport failed; evidence not judged, no Gate started',
                'The evidence was not judged. Retry this Actions attempt once GitHub transport recovers; '
                'a transport failure never requires a new local validation.'),
}
REJECTED = ('Evidence not admitted; validate locally',
            'Resolve the stated cause; a new complete local validation is needed only when the evidence itself '
            'was rejected.')


def rejection(run, identity, result):
    """(title, summary) of a rejected attempt."""
    title, recovery = FAILURES.get(result['category'], REJECTED)
    return title, (f"Commit: `{run['head_sha']}`; Actions attempt: `{identity}`\n\n"
                   f"No tests or Gate were run, and none starts automatically. {result['category']}: {result['error']}"
                   f"\n\n{recovery}")


def publish(run, config, home, identity, check_id, superseded):
    """Verify, pin and publish under the ledger's shared lock; any failure publishes a rejection."""
    prefix = '/repos/' + config['repository']
    job = Path(home) / 'jobs' / identity.replace('/', '-')
    reserved = []
    try:
        with ledger.hold(Path(config['publication_ledger'])):
            token = token_of(config)
            number = target(run, prefix, token)
            result, record = verify(run, config, home, identity, token, prefix, number)
            # Re-read head/base and the Actions attempt immediately before publication.
            if candidate(run, number, prefix, token) != {'base': result['base'], 'pull_request': result['pull_request']}:
                raise Rejected('head or base changed during verification')
            if superseded():
                raise Rejected('Actions attempt superseded before publication')
            # The unconfirmed publication lives in the pin until it is confirmed or dropped.
            value = reserve(config, record, result, identity)
            reserved.append(value['pin_id'])
            result = succeed(prefix, token, check_id, config, result | {'pin_id': value['pin_id']}, value)
            advance(home, result['ledger_head'])
        return result | {'status': 'PASS'}
    except Exception as error:
        result = {'status': 'REJECTED', 'category': getattr(error, 'category', type(error).__name__),
                  'error': str(error)[:4000]}
        (job / 'failure.json').write_text(json.dumps(result, indent=2) + '\n')
        # An unconfirmed attempt is never a merge source; its reservation is returned (after the pin lock is released).
        if reserved:
            pins.release(config['pins'], set(reserved))
        complete(prefix, installation_token(config), check_id, 'failure', *rejection(run, identity, result))
        return result


def merged_at_ms(pr):
    return calendar.timegm(time.strptime(pr['merged_at'], '%Y-%m-%dT%H:%M:%SZ')) * 1000


def settle(config, prefix, token, pin_id, value):
    """Hand a merged PR proof over to its main verification, or release a proof that can no longer be merged."""
    pr = api(prefix + '/pulls/' + str(value['subject']['pull_request']), token)
    if pr.get('merged') is True:
        # Main verification of the merge may start any time within the audit window after the merge.
        pins.extend(config['pins'], pin_id, merged_at_ms(pr) + config['audit_window_seconds'] * 1000)
        return []
    if pr.get('state') == 'open' and pr.get('head', {}).get('sha') == value['subject']['head']:
        return []
    return pins.release(config['pins'], {pin_id})


def sweep(config, token):
    """Between attempts (the service lock excludes a concurrent publication), settle every live pin.

    An unconfirmed pin belongs to an attempt that was interrupted and is never a merge
    source, so it is released. A confirmed PR proof is extended past its merge until its
    main verification can run, or released once its PR closed unmerged or moved head.
    Main publications keep their audit-window pin. Publications live only in their pins,
    so released or expired pins leave no records behind."""
    prefix = '/repos/' + config['repository']
    released = []
    for pin_id, value in sorted(pins.current(config['pins']).items()):
        if value['publication']['status'] != 'confirmed':
            released += pins.release(config['pins'], {pin_id})
        elif value['subject']['pull_request'] is not None:
            released += settle(config, prefix, token, pin_id, value)
    return released
