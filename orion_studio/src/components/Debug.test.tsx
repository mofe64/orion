import { renderToStaticMarkup } from "react-dom/server";
import { describe,expect,it,vi } from "vitest";
import { Debug } from "./Debug";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { GatewayStatus } from "../types";
const voice = { snapshot:{}, playback:{state:"idle"},label:"Off", settings:{model:"gpt-6-luna",effort:"medium",asrPath:"/pi/models/qwen"} } as StudioVoice;
const base = { runtime:{torque_enabled:true,mode:"ready",motion:null},character:{enabled:false},scene:{active:null} } as unknown as GatewayStatus;
function markup(status:GatewayStatus|null,connected=true) { return renderToStaticMarkup(<Debug voice={voice} status={status} connection={connected ? {url:"http://test",token:"test"}:null} onRefresh={vi.fn()} onHistory={vi.fn()}/>); }
describe("Debug torque release availability",()=>{
  it("describes the Pi voice source and keeps load errors inspectable", () => {
    expect(markup(base)).toContain("Runtime readings and voice activity reported by Orion’s Pi.");
    expect(markup(base)).not.toContain("local voice worker");
    expect(markup(base)).toContain("Rustpotter");
    expect(markup(base)).toContain("Silero VAD");
    expect(markup(base)).toContain("ASR files");
    expect(markup(base)).toContain("/pi/models/qwen");
    expect(markup(base)).toContain("gpt-6-luna");
  });
  it("offers release only for a connected stationary robot holding torque",()=>{ expect(markup(base)).toContain('class="quiet-button">Turn torque off</button>'); expect(markup(base,false)).toContain('disabled="">Turn torque off</button>'); });
  it("blocks release while moving, in character mode, or already off",()=>{ expect(markup({...base,character:{...base.character,enabled:true}})).toContain('disabled="">Turn torque off</button>'); expect(markup({...base,runtime:{...base.runtime,mode:"moving"}})).toContain('disabled="">Turn torque off</button>'); expect(markup({...base,runtime:{...base.runtime,torque_enabled:false}})).toContain('disabled="">Torque is off</button>'); });
});

it("shows plain labels alongside exact state tokens", () => {
  const html = markup({...base, runtime:{...base.runtime,mode:"holding"},character:{...base.character,state:"home_idle",active_clip:"glance",next_idle_category:"micro"}});
  for (const [label,token] of [["Holding position","holding"],["Idle at home","home_idle"],["Looking around","glance"],["Small idle gesture","micro"]]) {
    expect(html).toContain(label); expect(html).toContain(`<code>${token}</code>`);
  }
});
