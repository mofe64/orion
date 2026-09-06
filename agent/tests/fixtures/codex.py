#!/usr/bin/env python3
"""Deterministic App Server peer; never authenticates or invokes a model."""
import json
import sys
import uuid

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
        assert 'Do not use tools' in params['baseInstructions']
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
        # Ignore commentary and events from unrelated conversations or turns.
        for tid, uid, phase, value in [(thread_id, turn_id, 'commentary', 'Do not speak this'),
                                      ('other', turn_id, 'final_answer', 'Wrong thread'),
                                      (thread_id, 'stale', 'final_answer', 'Wrong turn')]:
            emit({'method':'item/completed', 'params': {'threadId':tid, 'turnId':uid,
                  'item': {'type':'agentMessage', 'phase':phase, 'text':value}}})
        response = '' if text == 'empty' else f'  Reply {turn}:\n{text}  '
        completed = {'method': 'turn/completed', 'params': {'threadId': thread_id,
                     'turn': {'id': turn_id, 'status': 'failed' if text == 'fail' else 'completed',
                              'error': {'message':'fixture failure'} if text == 'fail' else None,
                              'items': []}}}
        emit({'method':'item/completed', 'params': {'threadId':thread_id, 'turnId':turn_id,
              'item': {'type':'agentMessage', 'phase':'final_answer', 'text':response}}})
        emit(completed)
        continue
    emit({'id':request['id'], 'result':result})
