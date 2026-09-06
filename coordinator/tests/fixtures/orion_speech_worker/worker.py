"""Deterministic inference peer: no model libraries or network access."""
import json
import sys
import time

reader, writer = sys.stdin.buffer, sys.stdout.buffer

def emit(value, pcm=b''):
    writer.write(json.dumps(value).encode()+b'\n'+pcm)
    writer.flush()

config = json.loads(reader.readline())
assert set(config) == {'protocol', 'asr_model', 'tts_model'}
emit(dict(type='ready',protocol=1,asr=dict(provider='qwen3-asr',model='fixture'),tts=dict(provider='chatterbox-turbo',model='fixture')))
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
        count = 10 if 'long' in text or 'hang-tts' in text else 2
        for seq in range(count):
            if seq == 1 and 'tts-fail' in text:
                emit(dict(type='error',id=rid,message='fixture synthesis failed'))
                break
            if seq == 6 and 'hang-tts' in text:
                time.sleep(60)
            if 'long' in text:
                time.sleep(.07)
            pcm = b'\1\0' * 24000
            emit(dict(type='chunk',id=rid,sequence=seq,sampleRate=24000,samples=len(pcm)//2,
                      generationMs=70 if 'long' in text else 1,synthesisMs=(seq+1)*70),pcm)
        else:
            emit(dict(type='end',id=rid,sequence=count,synthesisMs=count*70))
