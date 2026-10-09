"""Deterministic inference peer: no model libraries or network access."""
import json
import sys
import time

reader, writer = sys.stdin.buffer, sys.stdout.buffer

def emit(value, pcm=b''):
    writer.write(json.dumps(value).encode()+b'\n'+pcm)
    writer.flush()

config = json.loads(reader.readline())
assert set(config) == {'protocol', 'role', 'asr_model', 'tts_model'}
tts_provider = 'piper-tts' if config['tts_model'] == 'piper-alba-medium' else 'chatterbox-turbo'
emit(dict(type='ready',protocol=2,role=config['role'],asr=dict(provider='qwen3-asr',model='fixture'),tts=dict(provider=tts_provider,model=config['tts_model'])))
for raw in reader:
    request = json.loads(raw)
    rid = request['id']
    if request['method'] == 'transcribe':
        text = reader.read(request['bytes']).decode().rstrip('\0')
        if 'hang-asr' in text:
            time.sleep(60)
        emit(dict(type='transcript',id=rid,text=text,language='English'))
    else:
        text = request['text']
        assert '[agree]' not in text and '[disagree]' not in text, 'Reaction tags must never reach TTS'
        assert len(text) <= 160, 'TTS inputs must be bounded before inference'
        if 'preplay-tts' in text:
            time.sleep(6)
        slow = 'slow-reply-tts' in text
        latched_burst = 'latched-burst-tts' in text or 'transition-burst-tts' in text
        transition = 'transition-burst-tts' in text
        over_limit = 'over-limit-tts' in text
        count = 1801 if over_limit else 50 if transition else 20 if slow or latched_burst else 10 if 'long' in text or 'hang-tts' in text else 2
        generation_ms = 1050 if slow else 1000 if latched_burst or over_limit else 70 if 'long' in text else 1
        for seq in range(count):
            if seq == 1 and 'tts-fail' in text:
                emit(dict(type='error',id=rid,message='fixture synthesis failed'))
                break
            if seq == 6 and 'hang-tts' in text:
                time.sleep(60)
            if slow:
                time.sleep(1.05)
            elif 'long' in text:
                time.sleep(.07)
            pcm = b'\1\0' * 24000
            emit(dict(type='chunk',id=rid,sequence=seq,sampleRate=24000,samples=len(pcm)//2,
                      generationMs=generation_ms,synthesisMs=(seq+1)*generation_ms),pcm)
        else:
            emit(dict(type='end',id=rid,sequence=count,synthesisMs=count*70))
