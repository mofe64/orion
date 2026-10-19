import {describe,it,expect,vi} from "vitest";
import {createOwnerLamp,ownerLampPreview} from "./ownerLamp";
import table from "../generated/lampEffects.json";
import type {LampStatus} from "../types";
const initial:LampStatus={on:true,brightness:40,mood:'warm',colors:[[40,20,4,210]],effect:'solid',takeover:null,fade:null};
describe('authoritative owner lamp',()=>{
  it('matches Rust owner frames for each effect, palette and hardware layout',()=>{
    for(const example of table.examples) {
      const setting:LampStatus={...initial,on:example.effect!=="off",effect:example.effect,brightness:65,colors:[[40,20,4,210],[255,0,0,0]]};
      const frame=ownerLampPreview(setting,example.time,example.count as 24|40);
      expect(frame.pixels?.map(p=>[p.red,p.green,p.blue,p.white]),`${example.effect} ${example.count} ${example.time}`).toEqual(example.pixels);
    }
  });
  it('rejects status captured before an edit and responses from a retired runtime',async()=>{
    const start={...initial,epoch:'one',revision:1};
    const control=createOwnerLamp(async()=>({...start,brightness:70,revision:3}),start);
    control.change({brightness:.7});
    for(let i=0;i<4;i++)await Promise.resolve();
    control.receive({...start,brightness:50,revision:2});
    expect(control.snapshot().setting?.brightness).toBe(70);
    control.receive({...start,brightness:20,revision:4});
    expect(control.snapshot().setting?.brightness).toBe(20);
    control.receive({...start,epoch:'two',brightness:100,revision:0});
    control.receive({...start,brightness:10,revision:9});
    expect(control.snapshot().setting?.brightness).toBe(100);
  });
  it('follows voice, other windows and reopening without substituting a takeover',()=>{
    const control=createOwnerLamp(vi.fn(),initial);
    const voice={...initial,brightness:65,colors:[[255,0,0,0]] as LampStatus['colors'],mood:'custom' as const,effect:'thinking_drift',takeover:'speech'};
    control.receive(voice);
    expect(control.snapshot().setting).toEqual(voice);
    const reopened=createOwnerLamp(vi.fn(),voice);
    expect(reopened.snapshot().setting).toEqual(voice);
    expect(ownerLampPreview(voice).red).toBeGreaterThan(0);
  });
  it('keeps a dragged slider and sends only brightness after a concurrent voice colour change',async()=>{
    let actual=initial;
    const send=vi.fn(async patch=>{actual={...actual,...patch,brightness:patch.brightness*100};return actual;});
    const control=createOwnerLamp(send,initial);
    control.begin();control.choose({brightness:.7});
    actual={...initial,brightness:30,colors:[[0,45,255,0]],mood:'custom'};
    control.receive(actual);
    expect(control.snapshot().setting?.brightness).toBe(70);
    expect(control.snapshot().setting?.colors).toEqual(actual.colors);
    expect(send).not.toHaveBeenCalled();
    control.release();await Promise.resolve();await Promise.resolve();
    expect(send).toHaveBeenCalledWith({brightness:.7});
    expect(control.snapshot().setting).toMatchObject({brightness:70,colors:actual.colors});
  });
  it('queues the newest edit and rolls back to lamp state on failure',async()=>{
    let finish!:(value:LampStatus)=>void;
    const send=vi.fn().mockImplementationOnce(()=>new Promise<LampStatus>(resolve=>{finish=resolve;})).mockRejectedValue(new Error('offline'));
    const control=createOwnerLamp(send,initial);
    control.change({brightness:.5});control.change({brightness:.8});
    expect(control.snapshot().setting?.brightness).toBe(80);
    finish({...initial,brightness:50});
    for(let i=0;i<6;i++)await Promise.resolve();
    expect(send).toHaveBeenCalledTimes(2);
    expect(control.snapshot().setting?.brightness).toBe(50);
    expect(control.snapshot().error).toContain("couldn't change the light");
  });
  it('renders the chosen palette, power and changing effect',()=>{
    expect(ownerLampPreview({...initial,on:false})).toMatchObject({red:0,green:0,blue:0,white:0});
    expect(ownerLampPreview(initial)).toMatchObject({red:16,green:8,blue:2,white:84});
    expect(ownerLampPreview({...initial,effect:'thinking_drift'},0)).not.toEqual(ownerLampPreview({...initial,effect:'thinking_drift'},1));
  });
});
