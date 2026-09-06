import { hueName, lampChannels } from "../lib/homeLamp";
export function StageColor({ label, value, onChange }: { label: string; value: string; onChange: (color: string) => void }) {
  const warm = value === "warm_white";
  const rgb = [1,3,5].map(index => parseInt(value.slice(index,index+2),16)/255);
  const max = Math.max(...rgb), min = Math.min(...rgb), delta = max-min;
  const hue = warm || !delta ? 210 : ((max === rgb[0] ? (rgb[1]-rgb[2])/delta : max === rgb[1] ? (rgb[2]-rgb[0])/delta+2 : (rgb[0]-rgb[1])/delta+4)*60+360)%360;
  const colorAt = (h: number) => "#" + lampChannels({ mood: "Custom color", brightness: 100, hue: h, enabled: true }).slice(0,3).map(channel => channel.toString(16).padStart(2,"0")).join("");
  return <fieldset className="stage-color"><legend>{label}</legend><div className="stage-color-modes"><button type="button" aria-pressed={warm} onClick={() => onChange("warm_white")}>Warm white</button><button type="button" aria-pressed={!warm} onClick={() => onChange(colorAt(210))}>Custom color</button></div>{!warm && <div className="oh-color"><input className="orion-hue-slider" aria-label={`${label} color`} aria-valuetext={hueName(hue)} type="range" min="0" max="359" value={hue} onChange={event => onChange(colorAt(Number(event.target.value)))} /><span className="oh-swatch" style={{ background: value }} /></div>}</fieldset>;
}
