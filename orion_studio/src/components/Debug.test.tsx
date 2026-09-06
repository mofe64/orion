import { renderToStaticMarkup } from "react-dom/server";
import { describe,expect,it,vi } from "vitest";
import { Debug } from "./Debug";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { GatewayStatus } from "../types";
const voice = { snapshot:{}, playback:{state:"idle"},label:"Off" } as StudioVoice;
const base = { runtime:{torque_enabled:true,mode:"ready",motion:null},character:{enabled:false},scene:{active:null} } as unknown as GatewayStatus;
function markup(status:GatewayStatus|null,connected=true) { return renderToStaticMarkup(<Debug voice={voice} status={status} connection={connected ? {url:"http://test",token:"test"}:null} onRefresh={vi.fn()}/>); }
describe("Debug torque release availability",()=>{
  it("offers release only for a connected stationary robot holding torque",()=>{ expect(markup(base)).toContain('class="quiet-button">Turn torque off</button>'); expect(markup(base,false)).toContain('disabled="">Turn torque off</button>'); });
  it("blocks release while moving, in character mode, or already off",()=>{ expect(markup({...base,character:{...base.character,enabled:true}})).toContain('disabled="">Turn torque off</button>'); expect(markup({...base,runtime:{...base.runtime,mode:"moving"}})).toContain('disabled="">Turn torque off</button>'); expect(markup({...base,runtime:{...base.runtime,torque_enabled:false}})).toContain('disabled="">Torque is off</button>'); });
});
