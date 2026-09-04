#!/usr/bin/env python3
"""Wait for the Pi listener and check authentication without opening capture."""
import asyncio
import json
from websockets.asyncio.client import connect
from websockets.exceptions import ConnectionClosed

async def main(url="ws://127.0.0.1:7448"):
    async with asyncio.timeout(20):
        while True:
            try:
                async with connect(url, open_timeout=2) as listener:
                    await listener.send(json.dumps(dict(type='hello', protocol=1, token='invalid-deployment-probe')))
                    try:
                        await listener.recv()
                    except ConnectionClosed as error:
                        if error.rcvd is not None and error.rcvd.code == 4003:
                            print('Pi listener is ready and rejects invalid authentication.')
                            return
                        raise
                    raise RuntimeError('Listener accepted an invalid token')
            except OSError:
                await asyncio.sleep(.2)

if __name__ == '__main__':
    asyncio.run(main())
