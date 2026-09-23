#!/usr/bin/env python3
"""Report gate results to GitHub via Checks API."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request
import urllib.error

def get_github_app_token():
    """Get GitHub App installation token from environment or credential helper."""
    # Option 1: From environment variable
    token = os.environ.get('GITHUB_APP_TOKEN')
    if token:
        return token

    # Option 2: From gh CLI (if available and authenticated as the app)
    try:
        result = subprocess.run(
            ['gh', 'auth', 'token'],
            capture_output=True,
            text=True,
            check=True
        )
        return result.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass

    return None

def create_check_run(repo_owner, repo_name, commit_sha, run_id, run_attempt, token):
    """Create a new check run for the commit."""
    url = f'https://api.github.com/repos/{repo_owner}/{repo_name}/check-runs'

    data = {
        'name': 'Trusted Harness-Gate',
        'head_sha': commit_sha,
        'status': 'in_progress',
        'external_id': f'{run_id}/{run_attempt}',
        'started_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
    }

    headers = {
        'Authorization': f'Bearer {token}',
        'Accept': 'application/vnd.github.v3+json',
        'Content-Type': 'application/json',
    }

    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode('utf-8'),
        headers=headers,
        method='POST'
    )

    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            result = json.loads(response.read().decode('utf-8'))
            return result['id']
    except urllib.error.HTTPError as e:
        print(f'Failed to create check run: {e.code} {e.reason}', file=sys.stderr)
        print(e.read().decode('utf-8'), file=sys.stderr)
        return None
    except urllib.error.URLError as e:
        print(f'Network error creating check run: {e.reason}', file=sys.stderr)
        return None

def update_check_run(repo_owner, repo_name, check_run_id, status, conclusion, summary, token, details_url=None):
    """Update an existing check run with results."""
    url = f'https://api.github.com/repos/{repo_owner}/{repo_name}/check-runs/{check_run_id}'

    data = {
        'status': status,
        'completed_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()) if status == 'completed' else None,
        'conclusion': conclusion if status == 'completed' else None,
        'output': {
            'title': 'Complete Local Isolated Gate',
            'summary': summary,
        }
    }

    if details_url:
        data['details_url'] = details_url

    # Remove None values
    data = {k: v for k, v in data.items() if v is not None}

    headers = {
        'Authorization': f'Bearer {token}',
        'Accept': 'application/vnd.github.v3+json',
        'Content-Type': 'application/json',
    }

    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode('utf-8'),
        headers=headers,
        method='PATCH'
    )

    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return True
    except urllib.error.HTTPError as e:
        print(f'Failed to update check run: {e.code} {e.reason}', file=sys.stderr)
        print(e.read().decode('utf-8'), file=sys.stderr)
        return False
    except urllib.error.URLError as e:
        print(f'Network error updating check run: {e.reason}', file=sys.stderr)
        return False

def report_gate_result(repo_path, commit_sha, run_id, run_attempt, success, run_dir, summary_text=None):
    """
    Report gate execution result to GitHub.

    Args:
        repo_path: Path to the repository
        commit_sha: Git commit SHA
        run_id: GitHub Actions run ID
        run_attempt: GitHub Actions run attempt number
        success: Whether the gate passed
        run_dir: Directory containing gate run artifacts
        summary_text: Optional summary text for the check
    """
    token = get_github_app_token()
    if not token:
        print('Warning: No GitHub App token available, skipping check report', file=sys.stderr)
        return False

    # Extract owner/repo from git remote
    try:
        result = subprocess.run(
            ['git', '-C', repo_path, 'remote', 'get-url', 'origin'],
            capture_output=True,
            text=True,
            check=True
        )
        remote_url = result.stdout.strip()

        # Parse GitHub owner/repo from URL
        # Support both HTTPS and SSH formats
        if 'github.com' in remote_url:
            parts = remote_url.replace('.git', '').replace(':', '/').split('/')
            repo_name = parts[-1]
            repo_owner = parts[-2]
        else:
            print(f'Warning: Non-GitHub remote: {remote_url}', file=sys.stderr)
            return False
    except subprocess.CalledProcessError:
        print('Warning: Failed to get git remote', file=sys.stderr)
        return False

    # Create check run
    check_run_id = create_check_run(repo_owner, repo_name, commit_sha, run_id, run_attempt, token)
    if not check_run_id:
        return False

    print(f'Created GitHub check run {check_run_id} for {commit_sha}', flush=True)

    # Prepare summary
    if summary_text is None:
        summary_text = f'''
## Gate Execution {'Passed ✓' if success else 'Failed ✗'}

**Run Directory:** `{run_dir}`
**Commit:** `{commit_sha[:7]}`
**Profile:** `ci`

The complete isolated gate execution has {'completed successfully' if success else 'failed'}.
All source files, runtime binaries, and policy configurations were verified.
'''

    # Update with result
    conclusion = 'success' if success else 'failure'
    updated = update_check_run(
        repo_owner,
        repo_name,
        check_run_id,
        'completed',
        conclusion,
        summary_text,
        token
    )

    if updated:
        print(f'Successfully reported {conclusion} to GitHub check {check_run_id}', flush=True)
    else:
        print(f'Failed to update GitHub check {check_run_id}', file=sys.stderr)

    return updated

if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--run-attempt', required=True)
    parser.add_argument('--success', action='store_true')
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--summary', type=str)
    args = parser.parse_args()

    success = report_gate_result(
        args.repository,
        args.commit,
        args.run_id,
        args.run_attempt,
        args.success,
        args.run_dir,
        args.summary
    )

    sys.exit(0 if success else 1)
