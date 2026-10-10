import { it, expect } from "vitest";
import { reminderRequest } from "./ReminderForm";
it("uses runtime reminder triggers and rejects incomplete clock input", () => {
  expect(reminderRequest("stretch", "", "09:00", ["mon"])).toEqual({action:"reminder",label:"stretch",repeat:{days:["mon"],time:"09:00"}});
  expect(() => reminderRequest("bins", "", "09:00", [])).toThrow("future date");
  expect(() => reminderRequest("", "", "09:00", ["mon"])).toThrow("1–200");
});
