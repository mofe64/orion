"""Sequential inference jobs over private parent/child pipes (protocol 1)."""
from __future__ import annotations
import json
import os
import sys
import time

MAX_LINE = 64 * 1024
MAX_PCM = 18 * 32000


def read_json(reader):
    raw = reader.readline(MAX_LINE + 1)
    if not raw:
        return None
    if len(raw) > MAX_LINE or not raw.endswith(b'\n'):
        raise ValueError('Invalid speech control frame')
    message = json.loads(raw)
    if not isinstance(message, dict):
        raise ValueError('Speech control must be an object')
    return message


def send(writer, message, pcm=None):
    writer.write(json.dumps(message, allow_nan=False).encode() + b'\n')
    if pcm is not None:
        writer.write(pcm)
    writer.flush()


def load_models(config):
    from .providers import Qwen3AsrTranscriber
    from .tts import ChatterboxSynthesizer
    return Qwen3AsrTranscriber(config['asr_model']), ChatterboxSynthesizer(config['tts_model'])


def serve(reader, writer, loader=load_models):
    config = read_json(reader)
    if config is None or config.get('protocol') != 1:
        raise ValueError('Unsupported speech protocol')
    asr, tts = loader(config)
    send(writer, dict(type='ready', protocol=1,
                     asr=dict(provider=asr.provider, model=asr.model_name),
                     tts=dict(provider=tts.provider, model=tts.model_name)))
    while (job := read_json(reader)) is not None:
        request_id = job.get('id')
        if type(request_id) is not int or request_id <= 0:
            raise ValueError('Invalid speech job ID')
        try:
            if job.get('method') == 'transcribe':
                size = job.get('bytes')
                if type(size) is not int or not 0 < size <= MAX_PCM or size % 2:
                    raise ValueError('Invalid PCM16 request')
                pcm = reader.read(size)
                if len(pcm) != size:
                    raise EOFError('Incomplete speech audio')
                result = asr.transcribe(pcm)
                send(writer, dict(type='transcript', id=request_id, text=result.text, language=result.language))
            elif job.get('method') == 'synthesize':
                text = job.get('text')
                if not isinstance(text, str) or not text.strip():
                    raise ValueError('Empty synthesis input')
                started = time.monotonic()
                stream = iter(tts.stream(text))
                sequence = total = 0
                try:
                    while True:
                        before = time.monotonic()
                        audio = next(stream, None)
                        generated = time.monotonic()
                        if audio is None:
                            break
                        if audio.sample_rate != 24000 or not audio.pcm or len(audio.pcm) % 2 or audio.samples > 48000:
                            raise ValueError('Invalid synthesis PCM16 chunk')
                        total += audio.samples
                        if total > 120 * 24000:
                            raise ValueError('Synthesized reply exceeds 120 seconds')
                        send(writer, dict(type='chunk', id=request_id, sequence=sequence,
                             sampleRate=24000, samples=audio.samples,
                             generationMs=(generated-before)*1000,
                             synthesisMs=(generated-started)*1000), audio.pcm)
                        sequence += 1
                finally:
                    if hasattr(stream, 'close'):
                        stream.close()
                if sequence == 0:
                    raise ValueError('Synthesis returned no audio')
                send(writer, dict(type='end', id=request_id, sequence=sequence,
                                 synthesisMs=(time.monotonic()-started)*1000))
            else:
                raise ValueError('Unknown speech job')
        except Exception as error:
            send(writer, dict(type='error', id=request_id, message=str(error)))
            # A failed job may leave unread bytes or native model state. The
            # coordinator starts a fresh worker rather than reuse uncertainty.
            return


def main():
    # Reserve stdout for framed IPC, including when native libraries print to fd 1.
    wire = os.fdopen(os.dup(sys.stdout.fileno()), 'wb')
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    try:
        serve(sys.stdin.buffer, wire)
    except Exception as error:
        send(wire, dict(type='error', message=str(error)))
        raise SystemExit(1)


if __name__ == '__main__':
    main()
