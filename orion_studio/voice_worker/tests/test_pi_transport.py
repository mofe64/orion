import asyncio
import json
import unittest
from websockets.asyncio.server import serve
from websockets.asyncio.client import connect
from orion_voice_worker.server import handle_connection, VoiceModels, validate_pi_url
from orion_voice_worker.providers import Transcript
from orion_voice_worker.tts import SpeechAudio

SID = 'a' * 32
HELLO = dict(type='hello', protocol=7, token='local')

class Asr:
    provider='qwen3-asr'; model_name='fake'
    def __init__(self, texts): self.texts=iter(texts)
    def transcribe(self, pcm): return Transcript(next(self.texts),'English')
class Agent:
    provider='test'; model_name='fake'
    def __init__(self): self.commands=[]
    def respond(self, text): self.commands.append(text); return 'Hello.'
class Tts:
    provider='chatterbox-turbo'; model_name='fake'
    def stream(self, text): yield SpeechAudio(b'\x00\x00'*240,24000)

class TransportTests(unittest.IsolatedAsyncioTestCase):
    async def exercise(self, texts, rejected=False, continuation=False):
        commands=[]
        agent=Agent()
        models=VoiceModels(Asr(texts),agent,Tts())
        async def pi(ws):
            current_sid = SID
            hello=json.loads(await ws.recv())
            self.assertEqual(hello,dict(type='hello',protocol=1,token='remote'))
            await ws.send(json.dumps(dict(type='ready',protocol=1,sampleRate=16000,channels=1,
                encoding='pcm_s16le',conversationWindow=continuation,wake=dict(provider='rustpotter',model='pi.rpw',threshold=.4))))
            await ws.send(json.dumps(dict(type='wake.candidate',sessionId=SID,name='hey_orion',score=.8)))
            async def utterance(purpose):
                await ws.send(json.dumps(dict(type='utterance',sessionId=current_sid,purpose=purpose,bytes=2)))
                await ws.send(b'\x00\x00')
            await utterance('wake_and_command')
            async for raw in ws:
                message=json.loads(raw); commands.append(message)
                self.assertEqual(message['sessionId'],current_sid)
                if message['type']=='wake.confirmed' and message['followup']:
                    await utterance('command')
                if continuation and message['type'] == 'session.finish' and current_sid == SID:
                    self.assertTrue(message['conversationWindow'])
                    await ws.send(json.dumps(dict(type='conversation.ready', sessionId=SID)))
                    current_sid = 'b' * 32
                    await ws.send(json.dumps(dict(type='command.candidate',sessionId=current_sid,previousSessionId=SID)))
                    await utterance('command')
        async with serve(pi,'127.0.0.1',0) as remote:
            url=f'ws://127.0.0.1:{remote.sockets[0].getsockname()[1]}'
            async def worker(ws):
                await handle_connection(ws,'local',models,url,'remote')
            async with serve(worker,'127.0.0.1',0) as local:
                async with connect(f'ws://127.0.0.1:{local.sockets[0].getsockname()[1]}') as client:
                    await client.send(json.dumps(HELLO))
                    events=[]
                    completions=0
                    async with asyncio.timeout(5):
                        while True:
                            raw=await client.recv()
                            if isinstance(raw,bytes):
                                self.assertEqual(len(raw),480)
                                continue
                            message=json.loads(raw); events.append(message['type'])
                            if message['type']=='speech.chunk': request_id=message['requestId']
                            if message['type']=='speech.end': await client.send(json.dumps(dict(type='playback.finished',requestId=request_id)))
                            if message['type'] == 'speech.completed': completions += 1
                            if message['type'] == 'wake.rejected' or completions >= (2 if continuation else 1): break
                        await client.send(json.dumps(dict(type='stop')))
        return commands,agent.commands,events

    async def test_confirmed_pi_audio_reaches_agent_and_playback(self):
        controls,commands,events=await self.exercise(['Hey Orion, what time is it?'])
        self.assertEqual(commands,['what time is it?'])
        self.assertEqual([m['type'] for m in controls],['wake.confirmed','session.processing','session.playing','session.finish'])
        self.assertIn('speech.completed',events)

    async def test_post_response_followup_uses_same_agent_and_fresh_voice_turn(self):
        controls, commands, events = await self.exercise(
            ['Hey Orion, tell me about Mars', 'How far away is it?'], continuation=True)
        self.assertEqual(commands, ['tell me about Mars', 'How far away is it?'])
        self.assertEqual(events.count('speech.completed'), 2)
        self.assertEqual(events.count('wake.candidate'), 1)
        self.assertIn('conversation.window', events)
        self.assertIn('command.started', events)
        self.assertEqual([m['sessionId'] for m in controls if m['type'] == 'session.finish'], [SID, 'b' * 32])

    async def test_qwen_false_positive_does_not_invoke_agent(self):
        controls,commands,events=await self.exercise(['That is an onion'],True)
        self.assertEqual(commands,[])
        self.assertEqual([m['type'] for m in controls],['session.reject'])
        self.assertNotIn('agent.started',events)

    async def test_paused_wake_and_followup_are_one_identified_session(self):
        controls,commands,_=await self.exercise(['Hey Orion','What time is it?'])
        self.assertEqual(commands,['What time is it?'])
        self.assertTrue(controls[0]['followup'])

    def test_lan_and_loopback_websocket_urls_are_accepted(self):
        for url in ['ws://orion.local:7448', 'ws://192.168.1.50:7448',
                    'ws://127.0.0.1:7448', 'ws://[::1]:7448']:
            with self.subTest(url=url):
                self.assertIsNone(validate_pi_url(url))

    def test_unsupported_schemes_and_embedded_credentials_are_rejected(self):
        for url in ['wss://orion.local:7448', 'http://orion.local:7448', 'ws:///missing-host',
                    'ws://user:password@orion.local', 'ws://orion.local?token=x',
                    'ws://orion.local#fragment']:
            with self.subTest(url=url), self.assertRaises(ValueError):
                validate_pi_url(url)
