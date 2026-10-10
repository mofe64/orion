import { expect,it } from "vitest";
import { debugStateLabel } from "./debugState";
it.each([["observe","Observing · torque off"],["holding","Holding position"],["home_idle","Idle at home"],["glance","Looking around"],["future_state","Future state"]])("labels %s without changing its token",(token,label) => expect(debugStateLabel(token)).toBe(label));
