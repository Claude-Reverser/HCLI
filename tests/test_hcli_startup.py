#!/usr/bin/env python3
"""Real-terminal HCLI regression check. Pass the freshly built executable path.

Uses an isolated home/socket and local model catalog; no live credentials or
model requests. Covers fresh setup, recovery from a deleted cwd, and key reuse.
"""
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

BINARY = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/jcode').resolve())


class Catalog(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({'data': [{'id': 'gpt-6-astra', 'object': 'model', 'extra': {'context': 1075200, 'pricing': {'currency': 'USD', 'input_per_million_usd': 0.4, 'output_per_million_usd': 2}}}]}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert request['model'] == 'gpt-6-astra', request['model']
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.end_headers()
        def event(payload):
            self.wfile.write(('data: ' + json.dumps(payload) + '\n\n').encode())
            self.wfile.flush()
        # No intermediate usage: reproduce gateways that report tokens only at EOF.
        for _ in range(40):
            event({'id': 'mock-response', 'choices': [{'index': 0, 'delta': {'content': 'Streaming test. '}, 'finish_reason': None}]})
            time.sleep(0.05)
        event({'id': 'mock-response', 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}], 'usage': {'prompt_tokens': 1000, 'completion_tokens': 320, 'total_tokens': 1320}})
        self.wfile.write(b'data: [DONE]\n\n')
        self.wfile.flush()

    def log_message(self, *args):
        pass


def launch(root, env, *, deleted_cwd=False, needs_key=False, extra_args=(), columns=140, rows=40, draft=None, stream=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))
    command_file = root / 'debug-command'
    response_file = root / 'debug-response'
    command_file.unlink(missing_ok=True)
    response_file.unlink(missing_ok=True)

    def send_debug(command):
        pending = command_file.with_suffix('.tmp')
        pending.write_text(command)
        pending.replace(command_file)
        if draft and draft.startswith('/'):
            os.kill(process.pid, signal.SIGWINCH)  # preserve picker selection
        else:
            os.write(master, b'\x1b[C')  # wake the idle event loop

    cwd = root / 'working'
    cwd.mkdir(exist_ok=True)

    def prepare_child():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        if deleted_cwd:
            os.rmdir(cwd)

    process = subprocess.Popen(
        [BINARY, '--no-update', '--no-selfdev', *extra_args],
        env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=prepare_child,
    )
    os.close(slave)
    output = b''
    sent_key = False
    sent_draft = False
    asked_state = False
    alternate_seen_at = None
    deadline = time.monotonic() + 45
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    output += os.read(master, 65536)
                except OSError:
                    break
            if needs_key and not sent_key and b'Enter your hcap.ai API key:' in output:
                if not termios.tcgetattr(master)[3] & termios.ICANON:
                    os.write(master, b'hcli-test-key\r')
                    sent_key = True
            if b'\x1b[?1049h' in output and alternate_seen_at is None:
                alternate_seen_at = time.monotonic()
            if alternate_seen_at is not None and time.monotonic() - alternate_seen_at > 1 and not asked_state:
                send_debug('state')
                asked_state = True
            if response_file.exists():
                response = response_file.read_text()
                if response.strip():
                    state = json.loads(response)
                    catalog_cache = root / 'cache' / 'hcap_models.json'
                    if state.get('model') == 'remote' or (state.get('provider_name') or '').lower() not in ('hcap', 'openrouter') or not catalog_cache.exists():
                        response_file.unlink(missing_ok=True)
                        send_debug('state')
                        continue
                    assert 'gpt-6-astra' in state['model'], state
                    cached_model = next(model for model in json.loads(catalog_cache.read_text())['models'] if model['id'] == 'gpt-6-astra')
                    assert cached_model['context_length'] == 1075200, cached_model
                    assert abs(float(cached_model['pricing']['prompt']) - 0.0000004) < 1e-18, cached_model
                    if draft is not None and not sent_draft:
                        os.write(master, draft.encode())
                        sent_draft = True
                        response_file.unlink(missing_ok=True)
                        send_debug('state')
                        continue
                    if draft is not None and state.get('input') != draft:
                        response_file.unlink(missing_ok=True)
                        send_debug('state')
                        continue
                    if draft is not None:
                        assert state['cursor_pos'] == len(draft.encode()), state
                    assert process.poll() is None, 'TUI exited after initialization'
                    assert b'launchctl' not in output and b'Unload failed' not in output
                    assert b'hcli-test-key' not in output, 'API key leaked to terminal'
                    if deleted_cwd:
                        assert b"working directory no longer exists" in output
                    assert (b'Enter your hcap.ai API key:' in output) == needs_key
                    # Check actual frame geometry, including wrapped drafts.
                    layout = None
                    layout_deadline = time.monotonic() + 8
                    response_file.unlink(missing_ok=True)
                    send_debug('layout')
                    while time.monotonic() < layout_deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            output += os.read(master, 65536)
                        if response_file.exists() and response_file.stat().st_size:
                            raw = response_file.read_text()
                            response_file.unlink(missing_ok=True)
                            if raw.startswith('{'):
                                layout = json.loads(raw)['layout']
                                break
                            os.kill(process.pid, signal.SIGWINCH)  # trigger a frame after enabling capture
                            send_debug('layout')
                    assert layout is not None, 'No rendered frame captured'
                    assert not layout['use_packed'], layout
                    box = layout['input_area']
                    assert box['y'] + box['height'] == rows - 2, layout
                    assert box['width'] == columns, layout
                    if draft is not None and len(draft) > columns:
                        assert layout['input_lines_wrapped'] > 1, layout
                    if stream:
                        def query(command):
                            response_file.unlink(missing_ok=True)
                            send_debug(command)
                            until = time.monotonic() + 5
                            while time.monotonic() < until:
                                if select.select([master], [], [], 0.05)[0]:
                                    os.read(master, 65536)
                                if response_file.exists() and response_file.stat().st_size:
                                    raw = response_file.read_text()
                                    response_file.unlink(missing_ok=True)
                                    return raw
                            raise AssertionError('No response to ' + command)
                        query('message:Reply with a short test response.')
                        seen_live = False
                        final_speed = None
                        until = time.monotonic() + 20
                        while time.monotonic() < until:
                            raw = query('screen-json')
                            if not raw.startswith('{'):
                                continue
                            frame = json.loads(raw)
                            speed = (frame.get('info_widgets') or {}).get('summary', {}).get('tokens_per_second')
                            if frame['state']['is_processing'] and speed and speed > 0:
                                seen_live = True
                            if not frame['state']['is_processing'] and frame['state']['message_count'] >= 2 and speed:
                                final_speed = speed
                                break
                        assert seen_live, 'No speed while streaming without intermediate usage'
                        assert final_speed and 100 < final_speed < 220, final_speed
                        time.sleep(0.3)
                        later = json.loads(query('screen-json'))['info_widgets']['summary']['tokens_per_second']
                        assert later == final_speed, (final_speed, later)
                        print('PASS: live estimated speed and retained final rate', round(final_speed, 1), 'tok/s')
                    print('PASS:', 'deleted cwd' if deleted_cwd else 'valid cwd',
                          'fresh key' if needs_key else 'saved key', 'live TUI:', state['model'])
                    return
            if process.poll() is not None:
                break
        raise AssertionError('TUI did not become ready; last state: ' + repr(locals().get('state')) + '; terminal: ' + repr(output[-1000:]))
    finally:
        daemon_pid = None
        metadata = Path(env['JCODE_SOCKET'] + '.server.json')
        if metadata.exists():
            try:
                daemon_pid = json.loads(metadata.read_text())['pid']
                os.kill(daemon_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        os.close(master)
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        if daemon_pid:
            until = time.monotonic() + 5
            while time.monotonic() < until:
                try:
                    os.kill(daemon_pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.05)
            else:
                os.kill(daemon_pid, signal.SIGKILL)


def main():
    catalog = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Catalog)
    threading.Thread(target=catalog.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='hcli-tui-', dir='/tmp') as directory:
        root = Path(directory)
        socket = root / 'runtime' / 'hcli.sock'
        assert not socket.parent.exists()
        env = dict(os.environ, JCODE_HOME=directory, JCODE_SOCKET=str(socket),
                   JCODE_RUNTIME_DIR=str(root / 'runtime'),
                   JCODE_NO_TELEMETRY='1', JCODE_NO_MENUBAR='1', TERM='xterm-256color',
                   JCODE_TEMP_SERVER='1', JCODE_SERVER_OWNER_PID=str(os.getpid()),
                   JCODE_DEBUG_CMD_PATH=str(root / 'debug-command'),
                   JCODE_DEBUG_RESPONSE_PATH=str(root / 'debug-response'))
        env.pop('HCAP_API_KEY', None)
        (root / 'config.toml').write_text(f'''[provider]
default_provider = "hcap"
default_model = "gpt-6-astra"
[providers.hcap]
type = "openai-compatible"
base_url = "http://127.0.0.1:{catalog.server_port}/v1"
api_key_env = "HCAP_API_KEY"
env_file = "hcap.env"
default_model = "gpt-6-astra"
model_catalog = true
''')
        try:
            launch(root, env, deleted_cwd=True, needs_key=True)
            launch(root, env)
            # Relative -C must be applied exactly once.
            (root / 'working' / 'project').mkdir()
            launch(root, env, extra_args=('-C', 'project'))
            launch(root, env, draft='/models')
            launch(root, env, columns=64, rows=24, draft='/models astra')
            launch(root, env, stream=True)
            launch(root, env, columns=64, rows=24, draft='1 Explain this project and help me improve its interface. ' * 3)
        finally:
            metadata = Path(env['JCODE_SOCKET'] + '.server.json')
            if metadata.exists():
                pid = json.loads(metadata.read_text())['pid']
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
    catalog.shutdown()


if __name__ == '__main__':
    main()
