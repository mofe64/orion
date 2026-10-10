import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { AlarmForm, alarmRequest } from "./AlarmForm";
import { LocationSettings } from "./LocationSettings";
describe("weather and briefing settings",()=>{
 it("creates a recurring briefing alarm",()=>{expect(alarmRequest("gym","","07:00",["mon"],true)).toEqual({action:"alarm",label:"gym",repeat:{days:["mon"],time:"07:00"},briefing:true});});
 it("validates sunrise minutes",()=>{expect(alarmRequest("wake","","07:00",["mon"],false,30).sunrise_minutes).toBe(30);expect(()=>alarmRequest("wake","","07:00",["mon"],false,4)).toThrow();});
 it("renders accessible controls and connection guidance",()=>{expect(renderToStaticMarkup(<AlarmForm disabled={false} onSave={async()=>true}/>)).toContain("Morning briefing after dismissal");expect(renderToStaticMarkup(<LocationSettings desktop={false} connected={false} connectionScope={null}/>)).toContain("Connect Orion");});
});
