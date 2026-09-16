#!/usr/bin/env python3
"""Deterministic App Server peer; never authenticates or invokes a model."""
import json
import sys
import uuid
import time

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
        assert {tool['name'] for tool in params['dynamicTools']} == {'append_memory', 'search_memories', 'set_lighting', 'set_mode', 'go_to_sleep', 'set_timer', 'set_alarm', 'list_alerts', 'cancel_alert', 'stop_alert'}
        assert params['config']['web_search'] == 'live'
        result = {'thread': {'id': thread_id}}
    elif method == 'turn/start':
        assert params['threadId'] == thread_id
        assert params['model'] == 'test-model'
        assert params['effort'] == 'high'
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
        if text == 'stream-fixture':
            for item, phase, tid, uid, value in [('comment', 'commentary', thread_id, turn_id, 'Never speak this. '), ('unknown', None, thread_id, turn_id, 'Unknown phase. '), ('stale', 'final_answer', thread_id, 'old', 'Wrong turn. '), ('final', 'final_answer', thread_id, turn_id, 'Let us begin. ')]:
                emit({'method':'item/started','params':{'threadId':tid,'turnId':uid,'item':{'type':'agentMessage','id':item,'phase':phase,'text':''}}})
                emit({'method':'item/agentMessage/delta','params':{'threadId':tid,'turnId':uid,'itemId':item,'delta':value}})
            time.sleep(.15)
            emit({'method':'item/agentMessage/delta','params':{'threadId':thread_id,'turnId':turn_id,'itemId':'final','delta':'Take a breath.'}})
            response = 'Let us begin. Take a breath.'
        completed = {'method': 'turn/completed', 'params': {'threadId': thread_id,
                     'turn': {'id': turn_id, 'status': 'failed' if text == 'fail' else 'completed',
                              'error': {'message':'fixture failure'} if text == 'fail' else None,
                              'items': []}}}
        emit({'method':'item/completed', 'params': {'threadId':thread_id, 'turnId':turn_id,
              'item': {'type':'agentMessage', 'phase':'final_answer', 'text':response}}})
        emit(completed)
        continue
    emit({'id':request['id'], 'result':result})
