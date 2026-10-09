#!/usr/bin/env python3
"""Deterministic App Server peer; never authenticates or invokes a model."""
import json
import sys
import uuid
import time
from datetime import datetime

thread_id = uuid.uuid4().hex
turn = 0

def emit(value):
    print(json.dumps(value), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    method, params = request.get('method'), request.get('params', {})
    result = {}
    if method == 'initialized':
        continue
    if method == 'account/read':
        result = {'account': {'type': 'chatgpt'}}
    elif method == 'model/list':
        result = {'data': [{'model': 'test-model', 'displayName': 'Test', 'hidden': False,
                           'supportedReasoningEfforts': [{'reasoningEffort': 'high'}]}], 'nextCursor': None}
    elif method == 'thread/start':
        assert params['approvalPolicy'] == 'never'
        assert params['sandbox'] == 'read-only'
        assert params['ephemeral'] is True
        assert 'set_lighting' in params['baseInstructions']
        assert {tool['name'] for tool in params['dynamicTools']} == {'append_memory', 'search_memories', 'get_lighting', 'set_lighting', 'set_mode', 'go_to_sleep', 'set_timer', 'set_alarm', 'list_alerts', 'cancel_alert', 'stop_alert'}
        assert params['config']['web_search'] == 'live'
        result = {'thread': {'id': thread_id}}
    elif method == 'turn/start':
        assert params['threadId'] == thread_id
        assert params['model'] == 'test-model'
        assert params['effort'] == 'high'
        context = params['input'][1]['text']
        assert context.startswith('Current UTC date/time: ')
        local = context.split('Current local date/time: ')[1].splitlines()[0]
        assert datetime.fromisoformat(local).tzinfo is not None
        assert 'IANA timezone: ' in context
        if params['input'][0]['text'] == 'start-rejected':
            emit({'id':request['id'],'error':{'code':-32602,'message':'fixture rejection'}})
            continue
        turn += 1
        turn_id = str(turn)
        text = params['input'][0]['text']
        if text == 'crash':
            sys.exit(1)
        emit({'id': request['id'], 'result': {'turn': {'id': turn_id}}})
        if text == 'hang':
            continue
        if text == 'approval':
            emit({'id': 'permission', 'method': 'item/commandExecution/requestApproval', 'params': {}})
            continue
        if text == 'unreadable':
            print('{invalid json', flush=True)
            continue
        if text in {'stale-tool', 'invalid-tool'}:
            emit({'id':'bad-tool','method':'item/tool/call','params':{'threadId':thread_id,
                  'turnId':'stale' if text == 'stale-tool' else turn_id, 'namespace':'invalid' if text == 'invalid-tool' else None,
                  'callId':'bad','tool':'list_alerts','arguments':{}}})
            continue
        if text == 'multiple-finals':
            for item in ['one','two']:
                emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,
                      'item':{'type':'agentMessage','id':item,'phase':'final_answer'}}})
            continue
        if text == 'multiple-completed-finals':
            for item in ['one','two']:
                emit({'method':'item/completed','params':{'threadId':thread_id,'turnId':turn_id,
                      'item':{'type':'agentMessage','id':item,'phase':'final_answer','text':item}}})
            continue
        if text in {'legacy-before-final', 'legacy-only'}:
            legacy = []
            for index in range(2):
                item = {'type':'agentMessage','id':f'legacy-{index}','text':f'Legacy {index}.'}
                legacy.append(item)
                emit({'method':'item/completed','params':{'threadId':thread_id,'turnId':turn_id,'item':item}})
            if text == 'legacy-only':
                emit({'method':'turn/completed','params':{'threadId':thread_id,'turn':{'id':turn_id,'status':'completed','items':legacy}}})
                continue
            emit({'id':'legacy-tool','method':'item/tool/call','params':{'threadId':thread_id,'turnId':turn_id,
                  'callId':'legacy-tool','tool':'search_memories','arguments':{'query':'nothing'}}})
            assert json.loads(sys.stdin.readline())['id'] == 'legacy-tool'
            final = {'type':'agentMessage','id':'final','phase':'final_answer','text':'Only the final.'}
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,'item':final}})
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,'itemId':'final','delta':final['text'] + ' '}})
            emit({'method':'item/completed','params':{'threadId':thread_id,'turnId':turn_id,'item':final}})
            emit({'method':'turn/completed','params':{'threadId':thread_id,'turn':{'id':turn_id,'status':'completed','items':legacy + [final]}}})
            continue
        if text in {'multiple-terminal-finals', 'repeated-terminal-final'}:
            items = [{'type':'agentMessage','id':item,'phase':'final_answer','text':'One answer.'}
                     for item in (['one', 'two'] if text == 'multiple-terminal-finals' else ['one', 'one'])]
            emit({'method':'turn/completed','params':{'threadId':thread_id,'turn':{'id':turn_id,'status':'completed','items':items}}})
            continue
        if text == 'tool-budget-fixture':
            results = []
            for index, call_id in enumerate([0, 0] + list(range(1, 17))):
                emit({'id':index,'method':'item/tool/call','params':{'threadId':thread_id,'turnId':turn_id,
                      'callId':str(call_id),'tool':'append_memory','arguments':{'text':f'Fact {call_id}'}}})
                reply = json.loads(sys.stdin.readline())
                assert reply['id'] == index
                results.append(reply['result'])
            text = json.dumps(results)
        if text == 'clock-fixture':
            text = context
        if text.startswith('tool:'):
            call = json.loads(text[5:])
            emit({'id':'tool-request','method':'item/tool/call','params':{'threadId':thread_id,'turnId':turn_id,'callId':'call-'+turn_id, 'tool':call['name'],'arguments':call['arguments']}})
            tool_result = json.loads(sys.stdin.readline())
            assert tool_result['id'] == 'tool-request'
            text = json.dumps(tool_result['result'])
        if text == 'search-fixture':
            for _ in range(2):
                emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,'item':{'type':'webSearch','id':'search','query':'restaurants in Woking'}}})
        # Ignore commentary and events from unrelated conversations or turns.
        for tid, uid, phase, value in [(thread_id, turn_id, 'commentary', 'Do not speak this'),
                                      ('other', turn_id, 'final_answer', 'Wrong thread'),
                                      (thread_id, 'stale', 'final_answer', 'Wrong turn')]:
            emit({'method':'item/completed', 'params': {'threadId':tid, 'turnId':uid,
                  'item': {'type':'agentMessage', 'phase':phase, 'text':value}}})
        response = '' if text == 'empty' else f'  Reply {turn}:\n{text}  '
        if text in {'changed-speech', 'empty-after-stream', 'tool-after-speech'}:
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,'itemId':'final','delta':'Original speech. '}})
            if text == 'tool-after-speech':
                emit({'id':'late-tool','method':'item/tool/call','params':{'threadId':thread_id,'turnId':turn_id,
                      'callId':'late','tool':'list_alerts','arguments':{}}})
                continue
            response = '' if text == 'empty-after-stream' else 'Changed speech.'
        if text == 'many-sentences-fixture':
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,
                  'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            pieces = ['hang-tts.'] + [f'Sentence {index}.' for index in range(40)]
            for piece in pieces:
                emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,
                      'itemId':'final','delta':piece + ' '}})
            response = ' '.join(pieces)
        if text == '800-prefix-fixture':
            prefix = 'a' * 799 + '.'
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,'itemId':'final','delta':prefix + ' '}})
            response = prefix + ' This ending survives.'
        if text == 'barge-in-fixture':
            prefix = 'First spoken sentence. Another spoken sentence. A third spoken sentence.'
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,
                  'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,
                  'itemId':'final','delta':prefix + ' '}})
            time.sleep(2)
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,
                  'itemId':'final','delta':'Unspoken tail. '}})
            response = prefix + ' Unspoken tail.'
        if text == 'stream-fixture':
            for item, phase, tid, uid, value in [('comment', 'commentary', thread_id, turn_id, 'Never speak this. '), ('unknown', None, thread_id, turn_id, 'Unknown phase. '), ('stale', 'final_answer', thread_id, 'old', 'Wrong turn. '), ('final', 'final_answer', thread_id, turn_id, 'Let us begin. ')]:
                emit({'method':'item/started','params':{'threadId':tid,'turnId':uid,'item':{'type':'agentMessage','id':item,'phase':phase,'text':''}}})
                emit({'method':'item/agentMessage/delta','params':{'threadId':tid,'turnId':uid,'itemId':item,'delta':value}})
            time.sleep(.15)
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,'itemId':'final','delta':'Take a breath.'}})
            response = 'Let us begin. Take a breath.'
        if text == 'reaction-cues-fixture':
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,
                  'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            for piece in ['[agr', 'ee] Yes. ', 'Fine. [sha', 'ke] No. ']:
                emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,
                      'itemId':'final','delta':piece}})
            response = '[agree] Yes. Fine. [shake] No. [agree]'
        if text == 'crowded-reaction-cues-fixture':
            response = '[agree] [happy] [surprised] [curious] [laugh] Yes!'
        if text == 'all-reaction-cues-fixture':
            names = ['agree', 'disagree', 'happy', 'curious', 'thinking', 'surprised', 'sympathy', 'unsure', 'laugh']
            emit({'method':'item/started','params':{'threadId':thread_id,'turnId':turn_id,
                  'item':{'type':'agentMessage','id':'final','phase':'final_answer'}}})
            sentences = [f'[{name.title()}] {name.title()}.' for name in names]
            for sentence in sentences:
                # Exercise split tags and case normalization across native deltas.
                for piece in [sentence[:3], sentence[3:] + ' ']:
                    emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,
                          'itemId':'final','delta':piece}})
            response = ' '.join(sentences)
        completed = {'method': 'turn/completed' , 'params': {'threadId': thread_id,
                     'turn': {'id': turn_id, 'status': 'failed' if text == 'fail' else 'completed',
                              'error': {'message':'fixture failure'} if text == 'fail' else None,
                              'items': []}}}
        emit({'method':'item/completed', 'params': {'threadId':thread_id, 'turnId':turn_id,
              'item': {'type':'agentMessage', 'phase':'final_answer', 'text':response}}})
        emit(completed)
        continue
    emit({'id':request['id'], 'result':result})
