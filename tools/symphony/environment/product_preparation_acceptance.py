"""Operator-owned acceptance of a reviewed product adapter; no caller arguments."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import time
import tomllib
import uuid
from preflight import ROOT
from execution_readiness import probe as readiness

BASE = Path(__file__).parent
FILES = ('app_server.py', 'sandbox_probe.py')


def digest(path):
    if path.is_symlink() or path.resolve() != path.absolute():
        raise ValueError('symlink in reviewed source path')
    return hashlib.sha256(path.read_bytes()).hexdigest()


def reviewed_sources():
    directory = BASE/'client/reviewed-preparation'
    manifest = json.loads((directory/'manifest.json').read_text())
    if set(manifest) != set(FILES):
        raise ValueError('unexpected preparation manifest')
    for name in FILES:
        expected = manifest[name]
        if digest(directory/name) != expected or digest(ROOT/'tools/preparation'/name) != expected:
            raise ValueError('product preparation source changed; operator review required: '+name)
    return directory, manifest


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate(result, expected_failure=None, unknown=False):
    if result['execution']['model_calls'] != 0:
        raise ValueError('model calls are forbidden in preparation acceptance')
    network = result['network']
    if not all(network[k] for k in ('allowed_probe','denied_probe','direct_connection_rejected')):
        raise ValueError('actual network boundary was not established')
    if unknown:
        if network['enforced'] or network['configuration_identity'] != 'unknown':
            raise ValueError('unknown configuration was accepted')
    elif not network['enforced']:
        raise ValueError('effective network identity mismatch')
    codes = [item['code'] for item in result['failures']]
    if codes != ([] if expected_failure is None else [expected_failure]):
        raise ValueError('unexpected preparation failures: '+repr(codes))


def probe():
    directory, hashes = reviewed_sources()
    ready = readiness()
    network = ready['requirements']['requirements']['network']
    configured = tomllib.loads((BASE/'requirements.toml').read_text())['experimental_network']
    domains = sorted(k for k,v in configured['domains'].items() if v == 'allow')
    if sorted(network['allowedDomains']) != domains:
        raise ValueError('installed/effective allowlist mismatch')
    adapter = load('reviewed_product_adapter', directory/'app_server.py')
    sampler = load('reviewed_product_sampler', directory/'sandbox_probe.py')
    config = dict(launcher=[str(BASE.parent/'codex-sandbox')], workspace=str(ROOT), uid=1000,
                  probe_path='/opt/symphony-env/reviewed-preparation/sandbox_probe.py',
                  allowed_url='https://index.crates.io/config.json', denied_url='https://example.com',
                  allowed_domains=domains, network_identity=adapter.identity(network),
                  deployment_identity=adapter.identity({'sources':hashes,'network':network}),
                  dependencies=[{'command':['rustc','--version'],'expected':'rustc '}],
                  writable_paths=[str(ROOT/'target')])
    cases = {}
    variants = [('ready', None), ('missing_dependency','preparation_dependency_missing'),
                ('wrong_capability','preparation_capability_mismatch'),
                ('wrong_version','preparation_capability_mismatch'),
                ('readonly_target','preparation_path_unwritable'), ('unknown_network',None)]
    for name, failure in variants:
        case = copy.deepcopy(config)
        if name == 'missing_dependency':
            case['dependencies']=[{'command':['/opt/symphony-env/absent-preparation-dependency'],'expected':'unused'}]
        elif name == 'wrong_capability':
            case['dependencies'][0]['expected']='required-capability-deliberately-unavailable'
        elif name == 'wrong_version':
            case['tool_lock']={'core_version':'harness-gate 0.0.0', 'core_sha256':sampler.CORE_SHA256,
                               'codex_version':sampler.CODEX_VERSION}
        elif name == 'readonly_target':
            case['writable_paths']=['/opt/symphony-env/reviewed-preparation']
        elif name == 'unknown_network':
            case['network_identity']='unknown'
        result = adapter.run(case)
        validate(result, failure, unknown=name=='unknown_network')
        cases[name]=result
    # Do not bless a workspace modified while the reviewed snapshot was running.
    if reviewed_sources()[1] != hashes:
        raise ValueError('reviewed source changed during acceptance')
    proof=dict(ok=True, sample_id=uuid.uuid4().hex, checked_at=time.time(),
               source_hashes={'tools/preparation/'+k:v for k,v in hashes.items()},
               workspace=str(ROOT), model_calls=0, cases=cases)
    destination=BASE/'client/product-preparation-acceptance.json'
    pending=destination.with_suffix('.new')
    pending.write_text(json.dumps(proof,indent=2)+'\n'); pending.replace(destination)
    return proof
