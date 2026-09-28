"""Install the accepted bounded retention entry with the existing approved schedule."""
import configparser
import json
from pathlib import Path
import subprocess

import bounded_layout as layout
import manual_measure
import rust_capture

HOME = Path('/home/gem/.local/share/codexsymphony/gate-host')
UNITS = Path('/home/gem/.config/systemd/user')
NAME = 'codexsymphony-bounded-retention'


def accepted_entry():
    approval = json.loads((HOME / 'approval.json').read_text())
    release = Path(approval['host_release'])
    entry = release / 'bounded_retention.py'
    if str(entry) not in approval['runtime_files']:
        raise ValueError('bounded retention is not in the installed approval')
    for name, digest in approval['runtime_files'].items():
        if Path(name).is_relative_to(release) and rust_capture.digest(Path(name)) != digest:
            raise ValueError('installed retention runtime changed')
    return entry


def approved_schedule():
    path = UNITS / 'codexsymphony-archive.timer'
    deployment = json.loads(manual_measure.DEPLOYMENT.read_text())
    if rust_capture.digest(path) != deployment['files'][str(path)]:
        raise ValueError('approved retention schedule changed')
    parser = configparser.ConfigParser(interpolation=None)
    parser.optionxform = str
    parser.read_string(path.read_text())
    schedule = dict(parser['Timer'])
    allowed = {'OnBootSec', 'OnUnitInactiveSec', 'OnCalendar', 'AccuracySec', 'RandomizedDelaySec', 'Persistent', 'Unit'}
    if set(schedule) - allowed:
        raise ValueError('unsupported retention timer configuration')
    schedule['Unit'] = NAME + '.service'
    return '\n'.join(key + '=' + value for key, value in schedule.items())


def install():
    layout.ensure(layout.REPOSITORY)
    entry = accepted_entry()
    service = '[Unit]\nDescription=Bounded fixed-workspace evidence retention\n[Service]\nType=oneshot\nExecStart=/usr/bin/python3 ' + str(entry) + ' --apply\nNice=10\n'
    timer = '[Unit]\nDescription=Bounded fixed-workspace evidence retention schedule\n[Timer]\n' + approved_schedule() + '\n[Install]\nWantedBy=timers.target\n'
    for suffix, content in (('service', service), ('timer', timer)):
        path = UNITS / (NAME + '.' + suffix)
        if path.is_symlink():
            raise ValueError('refuse to replace aliased unit')
        path.write_text(content)
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    subprocess.run(['systemctl', '--user', 'enable', '--now', NAME + '.timer'], check=True)
    return {'entry': str(entry), 'timer': NAME + '.timer', 'schedule': approved_schedule()}


if __name__ == '__main__': print(json.dumps(install(), indent=2))
