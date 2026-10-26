import { ChoiceSetting } from "./ChoiceSetting";
import type { DanceMode } from "../lib/gateway";
export const danceOptions: [DanceMode, string, string][] = [
  ["off", "Off", "Orion keeps its usual movements while music plays."],
  ["sometimes", "Sometimes", "Orion dances for a while, then takes a break."],
  ["always", "Always", "Orion dances whenever it hears music with a clear beat."],
];
export function DanceSetting({value, disabled, onChange}: {value?:DanceMode; disabled?:boolean; onChange:(mode:DanceMode)=>void}) {
  return <ChoiceSetting label="Dance to music" options={danceOptions} value={value} disabled={disabled} onChange={onChange}/>;
}
