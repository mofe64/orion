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
  expect(save).toHaveBeenCalledWith(connection, { curiosity: "shy" }, expect.anything());
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
