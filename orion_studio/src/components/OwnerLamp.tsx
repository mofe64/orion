import {useEffect,useMemo,useSyncExternalStore} from "react";
import {Sun,SunDim} from "lucide-react";
import type {LampColor,LampStatus} from "../types";
import {updateLamp,type GatewayConnection} from "../lib/gateway";
import {createOwnerLamp,lampMoods,lampEffects,lampColorHex} from "../lib/ownerLamp";

export function OwnerLamp({connection,status,resting,onPreview}:{connection:GatewayConnection;status?:LampStatus|null;resting:boolean;onPreview:(setting:LampStatus|null)=>void}) {
  const control=useMemo(()=>createOwnerLamp(patch=>updateLamp(connection,patch),status??null),[connection]);
  const view=useSyncExternalStore(control.subscribe,control.snapshot,control.snapshot);
  useEffect(()=>{control.receive(status);},[control,status]);
  useEffect(()=>()=>control.cancel(),[control]);
  useEffect(()=>onPreview(view.setting),[view.setting,onPreview]);
  const lamp=view.setting;
  const color=(index:number,hex:string)=>{
    const channels=[...hex.slice(1).match(/../g)!.map(c=>parseInt(c,16)),0] as LampColor;
    const colors=[...(lamp?.colors??[])];colors[index]=channels;
    control.change({colors,on:true});
  };
  return <section className="oh-lamp" aria-label="Lamp controls" aria-busy={view.pending}>
    <header className="oh-panel-heading"><h2>Lamp</h2><button className="oh-switch" type="button" role="switch" aria-label="Lamp power" aria-checked={lamp?.on??false} disabled={!lamp} onClick={()=>control.change({on:!lamp?.on})}><span/></button></header>
    <p id="lamp-command-state">{lamp ? `Light ${lamp.on ? "on" : "off"}` : "Update Orion to see and change its light settings."}</p>
    <div className="oh-dial"><svg viewBox="0 0 216 216" aria-hidden="true"><circle className="oh-dial-track" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray="429 572"/>{lamp && <circle className="oh-dial-value" cx="108" cy="108" r="91" fill="none" strokeWidth="8" strokeLinecap="round" strokeDasharray={`${429*lamp.brightness/100} 572`}/>}</svg><div><output htmlFor="lamp-brightness">{lamp ? <>{Math.round(lamp.brightness)}<small>%</small></> : "–"}</output><label htmlFor="lamp-brightness">Brightness</label></div></div>
    <div className="oh-range"><SunDim size={18}/><input id="lamp-brightness" type="range" min="0" max="100" value={lamp?.brightness??0} disabled={!lamp} aria-valuetext={lamp ? `${Math.round(lamp.brightness)}%` : "Light settings unavailable"} onPointerDown={control.begin} onChange={event=>control.choose({brightness:Number(event.target.value)/100,on:true})} onPointerUp={control.release} onPointerCancel={control.release} onKeyUp={control.release} onBlur={control.release}/><Sun size={18}/></div>
    <fieldset className="oh-light-moods"><legend>Light mood</legend><div>{Object.entries(lampMoods).map(([key,mood])=><button type="button" key={key} disabled={!lamp} aria-pressed={lamp?.mood===key} onClick={()=>control.change({colors:mood.colors,on:true})}>{mood.label}</button>)}<button type="button" disabled={!lamp} aria-pressed={lamp?.mood==='custom'} onClick={()=>control.change({colors:[[0,90,255,0]],on:true})}>Custom color</button></div></fieldset>
    {lamp && <div className="oh-palette">{lamp.colors.map((value,index)=><label key={index}>Color {lamp.colors.length>1 ? index+1 : ""}<input type="color" aria-label={`Lamp color ${index+1}`} value={lampColorHex(value)} onChange={event=>color(index,event.target.value)}/></label>)}{lamp.colors.length===1 ? <button className="oh-text-button" type="button" onClick={()=>control.change({colors:[lamp.colors[0],[255,0,0,0]]})}>Add a second color</button> : <button className="oh-text-button" type="button" onClick={()=>control.change({colors:[lamp.colors[0]]})}>Use one color</button>}</div>}
    <label className="oh-effect">Light effect<select aria-label="Light effect" value={lamp?.effect??'solid'} disabled={!lamp} onChange={event=>control.change({effect:event.target.value,on:true})}>{Object.entries(lampEffects).map(([key,label])=><option value={key} key={key}>{label}</option>)}</select></label>
    {lamp?.fade && <p className="oh-note" role="status">Fading to your setting · {Math.ceil(lamp.fade.remaining_seconds)} seconds left.</p>}
    {(lamp?.takeover || resting) && <p className="oh-note" role="status">{resting || lamp?.takeover==='rest' ? "Orion is resting. Your light setting is saved." : "Orion is using the light for a moment. Your setting is saved."}</p>}
    {view.error && <p className="oh-feedback" role="alert">{view.error}</p>}
    <p className="oh-note">Changes appear here when you ask Orion or use another device.</p>
  </section>;
}
