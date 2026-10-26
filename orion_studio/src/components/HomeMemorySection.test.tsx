import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { HomeMemorySection } from "./AgentProfileSettings";
import type { AgentProfile } from "../lib/agentProfile";

const profile = (patch: Partial<AgentProfile>) => ({ memories: [], ...patch }) as unknown as AgentProfile;
it("shows open things to come back to, habits and days, and hides finished follow-ups", () => {
  const html = renderToStaticMarkup(<HomeMemorySection blocked={false} confirming={false} onConfirming={() => {}} onChange={() => {}}
    profile={profile({ habits: ["People usually arrive around 09:00."], home: { followUps: [
      { id: "a", created: "", text: "Ask how the interview went", on: "2026-10-30", offered: 0, done: false },
      { id: "b", created: "", text: "Ask about the parcel", on: "2026-10-20", offered: 1, done: true },
    ], days: [{ day: "2026-10-25", text: "Sunday 25 October: someone stopped by at 10:00." }] } })} />);
  expect(html).toContain("Ask how the interview went");
  expect(html).toContain("only when you talk to it");
  expect(html).not.toContain("Ask about the parcel");
  expect(html).toContain("arrive around 09:00");
  expect(html).toContain("someone stopped by");
  expect(html).toContain("Forget everything about home");
});
it("explains each empty shelf in plain words", () => {
  const html = renderToStaticMarkup(<HomeMemorySection blocked={false} confirming={false} onConfirming={() => {}} onChange={() => {}} profile={profile({})} />);
  expect(html).toContain("still learning your habits");
  expect(html).not.toContain("Forget everything about home");
});
