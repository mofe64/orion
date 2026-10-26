import { expect, it, vi } from "vitest";
import { QuirkPreference } from "./quirkPreference";

const connection = { url: "http://orion", token: "test" };
it("reads Orion's quirks, saves one choice and restores it when saving fails", async () => {
  const read = vi.fn().mockResolvedValue({ quirks: { curiosity: "nosy", chattiness: "reserved" } });
  const save = vi.fn().mockResolvedValueOnce({ quirks: { curiosity: "shy", chattiness: "reserved" } }).mockRejectedValueOnce(new Error("offline"));
  const preference = new QuirkPreference(connection, { read, save });
  await preference.start();
  expect(preference.snapshot().values).toEqual({ curiosity: "nosy", chattiness: "reserved" });
  await preference.choose("curiosity", "shy");
  expect(save).toHaveBeenCalledWith(connection, { quirks: { curiosity: "shy" } }, expect.anything());
  expect(preference.snapshot()).toMatchObject({ values: { curiosity: "shy" }, notice: "Saved on Orion." });
  await preference.choose("chattiness", "chatty");
  expect(preference.snapshot().values?.chattiness).toBe("reserved");
  expect(preference.snapshot().error?.message).toContain("couldn't save");
});

it("rejects a personality it does not understand", async () => {
  const preference = new QuirkPreference(connection, { read: vi.fn().mockResolvedValue({ quirks: { curiosity: "loud" } }), save: vi.fn() });
  await preference.start();
  expect(preference.snapshot().values).toBeNull();
  expect(preference.snapshot().error?.message).toContain("couldn't read");
});

it("reads and saves Lively's rules beside the quirks", async () => {
  const rules = { quiet_from: 1320, quiet_until: 420, per_hour: 4, spacing_minutes: 10, after_talk_minutes: 5, welcome_after_minutes: 10,
    break_after_minutes: 90, goodnight_from: 1260, sleep_after_goodnight_minutes: 2, wave_back: true, light_care: true, break_nudge: true,
    goodnight: true, follow_ups: true, new_things: true, looks: true };
  const quirks = { curiosity: "nosy", chattiness: "reserved" };
  const save = vi.fn().mockResolvedValue({ quirks, rules: { ...rules, per_hour: 6 } });
  const preference = new QuirkPreference(connection, { read: vi.fn().mockResolvedValue({ quirks, rules }), save });
  await preference.start();
  await preference.chooseRules({ per_hour: 6 });
  expect(save).toHaveBeenCalledWith(connection, { rules: { per_hour: 6 } }, expect.anything());
  expect(preference.snapshot().rules?.per_hour).toBe(6);
  const older = new QuirkPreference(connection, { read: vi.fn().mockResolvedValue({ quirks }), save });
  await older.start();
  expect(older.snapshot()).toMatchObject({ values: quirks, rules: null });
});
