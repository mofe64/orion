import type { LampColor, LampStatus, LightPreview } from "../types";
import type { LampUpdate } from "./gateway";

export const lampMoods = {
  warm:{label:"Warm white",colors:[[40,20,4,210]]},
  ambient:{label:"Golden",colors:[[255,100,0,80]]},
  cool:{label:"Cool",colors:[[150,190,255,110],[0,45,255,0]]},
  warm_red:{label:"Warm red",colors:[[40,20,4,210],[255,0,0,0]]},
} satisfies Record<string,{label:string;colors:LampColor[]}>;
export const lampEffects:Record<string,string> = {solid:"Steady",warm_idle_breathe:"Gentle breathe",attentive_focus:"Focus",thinking_drift:"Drift",speaking_energy:"Soft pulse",acknowledge_pulse:"Pulse",curious_sweep:"Sweep",delight_spark:"Sparkle",settle_glow:"Glow"};

function project(actual:LampStatus|null, patch:LampUpdate):LampStatus|null {
  if (!actual) return null;
  return {...actual,...patch,brightness:patch.brightness===undefined ? actual.brightness : patch.brightness*100,
    ...(patch.colors ? {mood:"custom" as const} : {}),...(Object.keys(patch).length ? {fade:null} : {})};
}

/** Only edited fields are sent, so a brightness drag preserves a concurrent voice colour change. */
export function createOwnerLamp(send:(patch:LampUpdate)=>Promise<LampStatus>, initial:LampStatus|null=null) {
  let actual=initial, edits:LampUpdate={}, queued:LampUpdate={}, inflight:LampUpdate={}, busy=false, dragging=false, generation=0;
  let view={setting:initial,pending:false,error:""};
  const listeners=new Set<()=>void>();
  const retiredEpochs=new Set<string>();
  const receive=(state?:LampStatus|null)=>{
    if(!state || (state.epoch && retiredEpochs.has(state.epoch))) return;
    if(actual?.epoch===state.epoch && state.revision!==undefined && actual?.revision!==undefined && state.revision<actual.revision) return;
    if(actual?.epoch && state.epoch && state.epoch!==actual.epoch) retiredEpochs.add(actual.epoch);
    actual=state;publish();
  };
  const publish=(error=view.error) => {view={setting:project(actual,{...inflight,...queued,...edits}),pending:busy,error};listeners.forEach(listener=>listener());};
  const flush=async () => {
    if (busy || !Object.keys(queued).length) return;
    const patch=queued, id=generation; queued={}; inflight=patch; busy=true; publish("");
    try {
      const state=await send(patch);
      if (id!==generation) return;
      receive(state);
    } catch {
      if (id!==generation) return;
      publish("Studio couldn't change the light. Please try again.");
    } finally {
      if(id===generation) {busy=false;inflight={};publish();void flush();}
    }
  };
  return {
    snapshot:()=>view,
    subscribe:(listener:()=>void)=>{listeners.add(listener);return ()=>{listeners.delete(listener);};},
    receive,
    begin:()=>{dragging=true;},
    choose:(patch:LampUpdate)=>{edits={...edits,...patch};publish("");},
    release:()=>{dragging=false;queued={...queued,...edits};edits={};void flush();},
    change:(patch:LampUpdate)=>{queued={...queued,...patch};publish("");if(!dragging)void flush();},
    cancel:()=>{generation++;edits={};queued={};inflight={};busy=false;dragging=false;},
  };
}

export function ownerLampPreview(setting:LampStatus|null|undefined, seconds=0):LightPreview {
  if(!setting || (!setting.on && !setting.fade) || setting.effect==='off') return {red:0,green:0,blue:0,white:0};
  const colors=setting.fade?.colors ?? setting.colors;
  const brightness=(setting.fade?.brightness ?? setting.brightness)/100;
  const first=colors[0],last=colors[colors.length-1];
  const blend=setting.effect==='solid' ? .5 : .5+.5*Math.sin(seconds*.6);
  const gain=brightness*(setting.effect==='solid' ? 1 : .55+.35*Math.cos(seconds*2));
  const channels=first.map((value,index)=>(value+(last[index]-value)*blend)*gain);
  return {red:channels[0],green:channels[1],blue:channels[2],white:channels[3]};
}
export function lampColorHex([r,g,b,w]:LampColor):string {
  return '#'+[r+w,g+w*.84,b+w*.6].map(v=>Math.min(255,Math.round(v)).toString(16).padStart(2,'0')).join('');
}
