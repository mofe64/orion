#!/usr/bin/env python3
"""Run the authorized hardware tracking trials through the ordinary runtime APIs.

Idles use Unix-socket play; speech uses Piper worker protocol 2 and the ordinary
gateway WAV/stream routes. This moves the robot and plays audible speech.
"""
import argparse
import io
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import urllib.request
import wave
from experiment_safety import TemperatureGuard, latch_hold, stop_and_hold

TEXTS = [
    'Hello. I am checking how smoothly my lamp head follows a small movement.',
    'A lamp can express curiosity with a gentle tilt. The movement should be clear, calm, and easy to follow. Today we are measuring the difference between the planned motion and what the motors actually do.',
    'Here is the longer, streamed reply for the tracking measurement. Imagine a quiet workshop on a rainy afternoon. A small lamp looks toward a drawing on the desk, pauses to consider it, and turns back toward its owner. Its head leads the movement while the body follows gently behind. Some changes are tiny, so we need to measure them carefully rather than assume that every planned angle becomes visible motion. This experiment records the commands and the measured positions without changing the animation or the motor settings.',
]

# Keep the original three-trial profile unchanged. Repeated fixed prose gives
# approximately 60/120 seconds with the installed Alba voice; measure the WAV,
# rather than pretending a word count guarantees a particular duration.
LONG_TEXTS = [
    TEXTS[0],
    ' '.join([TEXTS[2]] * 2 + [TEXTS[0]] * 2),
    ' '.join([TEXTS[2]] * 4 + [TEXTS[0]] * 3),
    ' '.join([TEXTS[2]] * 2 + [TEXTS[0]] * 2),
]


def request(path, command):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(10)
        s.connect(str(path))
        s.sendall(command.encode() + b'\n')
        chunks = bytearray()
        while not chunks.endswith(b'\n'):
            data = s.recv(65536)
            if not data:
                break
            chunks.extend(data)
    response = json.loads(chunks)
    if command != 'status' and not response.get('ok'):
        raise RuntimeError(f'{command}: {response}')
    return response


def wait_movement(path, run=None, timeout=35):
    deadline = time.monotonic() + timeout
    temperature_guard = TemperatureGuard()
    while time.monotonic() < deadline:
        state = request(path, 'status')
        reasons = temperature_guard.observe(state,lambda row:print(json.dumps(row),flush=True))
        if reasons:
            stop_and_hold(lambda command:request(path,command))
            raise RuntimeError('; '.join(reasons))
        last = state.get('last_motion')
        if state.get('motion') is None and (run is None or last and last['run_id'] == run):
            return state
        time.sleep(.05)
    stop_and_hold(lambda command:request(path,command))
    raise TimeoutError(f'Movement {run} did not finish.')


def wav(pcm):
    data = io.BytesIO()
    with wave.open(data, 'wb') as writer:
        writer.setparams((1, 2, 24000, 0, 'NONE', ''))
        writer.writeframes(pcm)
    return data.getvalue()


def synthesize(args):
    env = os.environ.copy()
    env['PYTHONPATH'] = str(args.speech_root)
    env['ORION_SPEECH_BACKEND'] = 'pi'
    env['ORION_PIPER_MODEL_DIR'] = str(args.piper_model)
    with open(args.output / 'tts-worker.log', 'wb') as errors:
        worker = subprocess.Popen([str(args.speech_python), '-m', 'orion_speech_worker.worker'],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, env=env)
        def send(value):
            worker.stdin.write(json.dumps(value).encode() + b'\n')
            worker.stdin.flush()
        def receive():
            data = worker.stdout.readline()
            if not data:
                raise RuntimeError('TTS worker ended; inspect tts-worker.log.')
            return json.loads(data)
        try:
            send({'protocol': 2, 'role': 'tts', 'tts_model': 'piper-alba-medium'})
            ready = receive()
            if ready.get('type') != 'ready':
                raise RuntimeError(f'TTS not ready: {ready}')
            utterances = []
            for job, text in enumerate(args.texts, 1):
                send({'id': job, 'method': 'synthesize', 'text': text})
                chunks = []
                while True:
                    row = receive()
                    if row.get('type') == 'end':
                        break
                    if row.get('type') != 'chunk' or row.get('id') != job:
                        raise RuntimeError(f'Unexpected TTS frame: {row}')
                    pcm = worker.stdout.read(row['samples'] * 2)
                    if len(pcm) != row['samples'] * 2:
                        raise RuntimeError('Incomplete TTS audio.')
                    chunks.append(pcm)
                duration = sum(map(len, chunks)) / 48000
                if args.long_speech and not 0 < duration <= 120:
                    raise ValueError(f'TTS {job} is {duration:.2f}s; gateway limit is 120s.')
                utterances.append(chunks)
                (args.output / f'utterance-{job}.wav').write_bytes(wav(b''.join(chunks)))
                print(f'TTS {job}: {sum(map(len,chunks))/48000:.2f} seconds, {len(chunks)} chunks', flush=True)
            return utterances
        finally:
            worker.stdin.close()
            try:
                worker.wait(timeout=10)
            except subprocess.TimeoutExpired:
                worker.kill()
                worker.wait()


def main():
    global request
    unguarded_request = request
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', type=Path, default=Path('/tmp/oriond.sock'))
    parser.add_argument('--motions', type=Path, default=Path('motion/motions/v1/idle'))
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--idles-only', action='store_true')
    parser.add_argument('--speech-only', action='store_true')
    parser.add_argument('--long-speech', action='store_true',
                        help='Short baseline, two complete WAVs (~60/~120s), and an ordered ~60s stream.')
    parser.add_argument('--speech-python', type=Path)
    parser.add_argument('--speech-root', type=Path)
    parser.add_argument('--piper-model', type=Path)
    parser.add_argument('--token-file', type=Path, default=Path.home()/'.config/orion/studio-token')
    parser.add_argument('--gateway-url', default='http://127.0.0.1:7447')
    args = parser.parse_args()
    if args.repeats < 1 or args.idles_only and args.speech_only:
        parser.error('Use positive repeats and select at most one subset.')
    if not args.idles_only and not all([args.speech_python, args.speech_root, args.piper_model]):
        parser.error('Speech needs --speech-python, --speech-root and --piper-model.')
    args.texts = LONG_TEXTS if args.long_speech else TEXTS
    stream_number = 4 if args.long_speech else 3
    args.output.mkdir(parents=True, exist_ok=True)
    utterances = synthesize(args) if not args.idles_only else []
    journal = open(args.output/'trials.jsonl', 'a', buffering=1)
    def record(value):
        journal.write(json.dumps(value) + '\n')
        print(json.dumps(value), flush=True)
    temperature_guard = TemperatureGuard()
    def guarded_request(path, command):
        response = unguarded_request(path, command)
        if command == 'status':
            reasons = temperature_guard.observe(response,record)
            if reasons:
                raise RuntimeError('; '.join(reasons))
        return response
    request = guarded_request
    try:
        if not args.speech_only:
            request(args.socket, 'character stop')
            wait_movement(args.socket)
            names = []
            for path in sorted(args.motions.glob('*.yaml')):
                found = re.search(r'^\s+name:\s*(\S+)', path.read_text(), re.MULTILINE)
                if not found:
                    raise ValueError(f'Missing motion name: {path}')
                names.append(found.group(1))
            for name in names:
                for repeat in range(1, args.repeats + 1):
                    home = request(args.socket, 'goto home 3.0')
                    home_state = wait_movement(args.socket, home['run_id'])
                    if home_state['last_motion']['state'] != 'completed':
                        raise RuntimeError(f'Home failed: {home_state["last_motion"]}')
                    play = request(args.socket, f'play {name}')
                    final = wait_movement(args.socket, play['run_id'])
                    record({'kind':'idle_trial','name':name,'repeat':repeat,'run_id':play['run_id'],
                            'terminal':final['last_motion'],'home_measured':home_state['joints']})
        if utterances:
            token = args.token_file.read_text().strip()
            def post(path, data, content_type, identifier):
                req = urllib.request.Request(args.gateway_url + path, data=data,
                    headers={'Authorization':'Bearer '+token, 'Content-Type':content_type,
                             'X-Orion-Voice-Request-ID':identifier})
                with urllib.request.urlopen(req, timeout=15) as response:
                    return json.load(response)
            request(args.socket, 'character start')
            wait_movement(args.socket)
            for number, chunks in enumerate(utterances, 1):
                request(args.socket, 'character state neutral')
                wait_movement(args.socket)
                # Restore the same home anchor between utterances through normal character control.
                request(args.socket, 'character stop')
                wait_movement(args.socket)
                request(args.socket, 'character start')
                wait_movement(args.socket)
                identifier = f'tracking-{time.time_ns()}-{number}'
                started = time.monotonic()
                def clock(stage, speech_state=None):
                    if args.long_speech:
                        snapshot = request(args.socket, 'status')
                        speech_state = speech_state or request(args.socket, 'speech status')
                        journal.write(json.dumps({'kind': 'speech_clock', 'number': number,
                            'stage': stage, 'sequence': snapshot['sequence'],
                            'sampled_at_unix_ns': snapshot['sampled_at_unix_ns'],
                            'motion': snapshot.get('motion'), 'speech': speech_state}) + '\n')
                clock('before_upload')
                print(f'Speech {number} starting: {sum(map(len,chunks))/48000:.2f}s, '
                      f'{"stream" if number == stream_number else "complete WAV"}', flush=True)
                if number != stream_number:
                    response = post('/api/v2/speech', wav(b''.join(chunks)), 'audio/wav', identifier)
                else:
                    response = post('/api/v2/speech/stream', wav(chunks[0]), 'audio/wav', identifier)
                    run = response['run_id']
                    clock('stream_started')
                    for sequence, chunk in enumerate(chunks[1:], 1):
                        time.sleep(.5)
                        post(f'/api/v2/speech/{run}/chunks/{sequence}', wav(chunk), 'audio/wav', identifier)
                        clock(f'chunk_{sequence}')
                    post(f'/api/v2/speech/{run}/end', json.dumps({'sequence':len(chunks)}).encode(), 'application/json', identifier)
                clock('upload_finished')
                run = response['run_id']
                deadline = time.monotonic() + 140
                while time.monotonic() < deadline:
                    request(args.socket, 'status')  # Thermal checks also cover short speech replies.
                    state = request(args.socket, 'speech status')
                    clock('playback', state)
                    terminal = state.get('last_speech')
                    if terminal and terminal['run_id'] == run and terminal['state'] in {'completed','failed','cancelled'}:
                        break
                    time.sleep(.1)
                else:
                    raise TimeoutError(f'Speech {run} did not finish.')
                motion = wait_movement(args.socket)
                record({'kind':'speech_trial','number':number,'streamed':number==stream_number,'speech_run_id':run,
                        'audio_seconds':sum(map(len,chunks))/48000,'wall_seconds':time.monotonic()-started,
                        'text':args.texts[number-1], 'speech_terminal':terminal, 'motion_terminal':motion.get('last_motion')})
                if terminal['state'] != 'completed':
                    raise RuntimeError(f'Speech failed: {terminal}')
    except BaseException as error:
        latch_hold(lambda command:unguarded_request(args.socket,command),Path.cwd(),str(error),record)
        raise
    finally:
        request = unguarded_request
        journal.close()



if __name__ == '__main__':
    main()
