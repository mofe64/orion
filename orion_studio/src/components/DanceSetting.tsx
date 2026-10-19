import {useId} from "react";
import type { DanceMode } from "../lib/gateway";
export const danceOptions: [DanceMode, string, string][] = [
  ["off", "Off", "Orion keeps its usual movements while music plays."],
  ["sometimes", "Sometimes", "Orion dances for a while, then takes a break."],
  ["always", "Always", "Orion dances whenever it hears music with a clear beat."],
];
export function DanceSetting({value, disabled, onChange}: {value?:DanceMode; disabled?:boolean; onChange:(mode:DanceMode)=>void}) {
  const group=useId();
  return <fieldset className="dance-setting" disabled={disabled || value===undefined}><legend>Dance to music</legend>
    {danceOptions.map(([mode,label,help])=><label key={mode}><input type="radio" name={group} value={mode} checked={value===mode} onChange={()=>onChange(mode)}/><span><strong>{label}</strong><small>{help}</small></span></label>)}
  </fieldset>;
}
