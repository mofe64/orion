import { useState } from "react";
import { reminderRequest, WEEKDAYS } from "./ReminderForm";
export function alarmRequest(label:string,date:string,time:string,days:string[],briefing:boolean,sunrise:number|null=null):Record<string,unknown> {
  if(label.length>80 || /[\x00-\x1f]/.test(label)) throw new Error("Use at most 80 printable characters.");
  if(sunrise!==null && (!Number.isInteger(sunrise) || sunrise<5 || sunrise>60)) throw new Error("Sunrise must be 5–60 minutes.");
  return {...reminderRequest(label.trim()||"Alarm",date,time,days),action:"alarm",label:label.trim(),briefing,...(sunrise!==null?{sunrise_minutes:sunrise}:{})};
}
export function AlarmForm({disabled,onSave}:{disabled:boolean;onSave:(request:Record<string,unknown>)=>Promise<boolean>}) {
 const [label,setLabel]=useState("");const [date,setDate]=useState("");const [time,setTime]=useState("07:00");const [days,setDays]=useState<string[]>([]);const [briefing,setBriefing]=useState(false);const [sunrise,setSunrise]=useState<number|null>(null);const [error,setError]=useState("");
 return <form className="oh-reminder-form" aria-label="New alarm" onSubmit={async e=>{e.preventDefault();setError("");try{if(await onSave(alarmRequest(label,date,time,days,briefing,sunrise)))setLabel("");}catch(e){setError(e instanceof Error?e.message:String(e));}}}>
  <div className="oh-form-fields">
   <label className="oh-field-wide">Label (optional)<input maxLength={80} value={label} disabled={disabled} onChange={e=>setLabel(e.target.value)} /></label>
   {!days.length && <label>Date (this device’s time zone)<input type="date" required value={date} disabled={disabled} onChange={e=>setDate(e.target.value)} /></label>}
   <label>Time{days.length?" (Orion’s local time)":""}<input type="time" required value={time} disabled={disabled} onChange={e=>setTime(e.target.value)} /></label>
   <label>Sunrise light<select disabled={disabled} value={sunrise??""} onChange={e=>setSunrise(e.target.value?Number(e.target.value):null)}><option value="">Off</option>{[5,10,15,20,30,45,60].map(m=><option key={m} value={m}>{m} minutes before</option>)}</select></label>
  </div>
  <fieldset className="oh-days" disabled={disabled}><legend>Repeat days (optional)</legend>{WEEKDAYS.map(day=><label key={day}><input type="checkbox" checked={days.includes(day)} onChange={e=>setDays(e.target.checked?[...days,day]:days.filter(d=>d!==day))}/><span>{day}</span></label>)}</fieldset>
  <div className="oh-form-footer"><label className="oh-check"><input type="checkbox" checked={briefing} disabled={disabled} onChange={e=>setBriefing(e.target.checked)}/>Morning briefing after dismissal</label><button className="primary-button" type="submit" disabled={disabled}>Save alarm</button></div>
  {error && <p role="alert" className="settings-error">{error}</p>}
 </form>;
}
