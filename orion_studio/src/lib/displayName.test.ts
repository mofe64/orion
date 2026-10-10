import { describe, expect, it } from "vitest";
import { assetDisplayName } from "./displayName";

describe("asset display names", () => {
  it.each([
    ["look_at_left_expressive", "Look at left expressive"],
    ["acknowledge_left", "Acknowledge left"],
    ["home", "Home"],
    ["", ""],
  ])("displays %s in sentence case", (id, expected) => {
    expect(assetDisplayName(id)).toBe(expected);
  });
});
