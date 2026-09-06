import io
import json
import unittest
from types import SimpleNamespace
from orion_speech_worker.worker import serve


class Asr:
    provider='qwen3-asr'; model_name='fake'
    def __init__(self): self.calls=[]
    def transcribe(self, pcm):
        self.calls.append(pcm)
        return SimpleNamespace(text='Hey Orion', language='English')


class Tts:
    provider='chatterbox-turbo'; model_name='fake'
    def stream(self, text):
        yield SimpleNamespace(pcm=b'\1\0'*2, samples=2, sample_rate=24000)
        if text=='fail': raise RuntimeError('inference failed')
        yield SimpleNamespace(pcm=b'\2\0'*2, samples=2, sample_rate=24000)


def control(value): return json.dumps(value).encode()+b'\n'


class WorkerTests(unittest.TestCase):
    def run_worker(self, jobs):
        reader=io.BytesIO(control(dict(protocol=1,asr_model='fake',tts_model='fake'))+jobs)
        writer=io.BytesIO(); asr=Asr()
        serve(reader,writer,lambda config:(asr,Tts()))
        output=io.BytesIO(writer.getvalue()); messages=[]
        while raw:=output.readline():
            value=json.loads(raw)
            if value['type']=='chunk': value['pcm']=output.read(value['samples']*2)
            messages.append(value)
        return messages,asr

    def test_transcription_and_synthesis_have_separate_identified_jobs(self):
        messages,asr=self.run_worker(control(dict(method='transcribe',id=1,bytes=2))+b'\0\0'+control(dict(method='synthesize',id=2,text='Hello')))
        self.assertEqual([v['type'] for v in messages],['ready','transcript','chunk','chunk','end'])
        self.assertEqual(messages[1]['id'],1)
        self.assertEqual([v['id'] for v in messages[2:]],[2,2,2])
        self.assertEqual([v['sequence'] for v in messages[2:]],[0,1,2])
        self.assertEqual(messages[2]['pcm'],b'\1\0'*2)
        self.assertEqual(asr.calls,[b'\0\0'])

    def test_failed_inference_ends_worker_without_processing_another_job(self):
        messages,_=self.run_worker(control(dict(method='synthesize',id=1,text='fail'))+control(dict(method='synthesize',id=2,text='later')))
        self.assertEqual([v['type'] for v in messages],['ready','chunk','error'])
        self.assertEqual(messages[-1]['id'],1)

    def test_rejects_oversized_odd_and_truncated_pcm(self):
        for size,pcm in [(18*32000+2,b''),(1,b'\0'),(4,b'\0\0')]:
            messages,asr=self.run_worker(control(dict(method='transcribe',id=1,bytes=size))+pcm)
            self.assertEqual(messages[-1]['type'],'error')
            self.assertEqual(asr.calls,[])

    def test_empty_synthesis_and_agent_jobs_are_rejected(self):
        for job in [dict(method='synthesize',id=1,text=''),dict(method='respond',id=1,text='hello')]:
            messages,_=self.run_worker(control(job))
            self.assertEqual(messages[-1]['type'],'error')

    def test_model_loader_runs_once_for_multiple_requests(self):
        calls=[]
        def load(config): calls.append(config); return Asr(),Tts()
        data=control(dict(protocol=1,asr_model='fake',tts_model='fake'))
        for rid in [1,2]: data+=control(dict(method='transcribe',id=rid,bytes=2))+b'\0\0'
        serve(io.BytesIO(data),io.BytesIO(),load)
        self.assertEqual(len(calls),1)


if __name__=='__main__': unittest.main()
