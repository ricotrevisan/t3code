import json, os, pathlib, selectors, shutil, subprocess, time
root = pathlib.Path(__file__).resolve().parent
(root / 'isolated-agent').mkdir(exist_ok=True)
(root / 'isolated-workspace').mkdir(exist_ok=True)
env = {**os.environ, 'PI_CODING_AGENT_DIR': str(root / 'isolated-agent')}
pi_command = [shutil.which('node'), os.environ['PI_REPRO_CLI']] if 'PI_REPRO_CLI' in os.environ else [os.environ.get('PI_REPRO_BINARY', shutil.which('pi'))]
p = subprocess.Popen([*pi_command, '--mode', 'rpc', '--no-session', '--no-extensions', '--no-skills', '--no-prompt-templates', '--no-themes', '--extension', str(root / 'startup-consent-fixture.mjs')], cwd=root / 'isolated-workspace', env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
sel = selectors.DefaultSelector(); sel.register(p.stdout, selectors.EVENT_READ)
def send(msg):
    p.stdin.write(json.dumps(msg) + '\n'); p.stdin.flush()
def read_event(seconds):
    if not sel.select(seconds): return None
    line = p.stdout.readline()
    if not line: raise RuntimeError('Pi process exited before fixture event')
    return json.loads(line)
try:
    send({'id':'health-fixture', 'type':'get_commands'})
    events = []
    deadline = time.monotonic() + 10
    request = None
    while time.monotonic() < deadline:
        event = read_event(1)
        if event:
            events.append(event)
            if event.get('type') == 'extension_ui_request': request = event; break
    assert request and request.get('method') == 'confirm', 'Real Pi must request startup consent'
    print('REAL_PI_STARTUP_UI_REQUEST: confirm')
    assert not any(e.get('id') == 'health-fixture' and e.get('type') == 'response' for e in events)
    event = read_event(0.5)
    assert not event or event.get('id') != 'health-fixture', 'get_commands must wait for startup consent'
    print('GET_COMMANDS_BLOCKED_BEFORE_APPROVAL: yes')
    send({'type':'extension_ui_response','id':request['id'],'confirmed':False})
    deadline = time.monotonic() + 10
    response = None
    while time.monotonic() < deadline:
        event = read_event(1)
        if event and event.get('type') == 'response' and event.get('id') == 'health-fixture': response = event; break
    assert response and response.get('success'), 'get_commands must resume after denial'
    assert any(c.get('name') == 't3-preview-health' and c.get('source') == 'extension' for c in response.get('data',{}).get('commands',[]))
    print('GET_COMMANDS_AFTER_DENIAL: success; t3-preview-health present')
finally:
    p.stdin.close()
    try: p.wait(timeout=3)
    except subprocess.TimeoutExpired:
        p.terminate()
        try: p.wait(timeout=3)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
    sel.close()
