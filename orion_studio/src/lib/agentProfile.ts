import { invoke } from "@tauri-apps/api/core";
import { forOwner } from "./feedback";
export interface Personality { traits: string[]; behaviors: string[] }
export interface PersonalityChoice { id: string; label: string; description: string }
export interface MemoryEntry { id: string; created: string; text: string }
export interface AgentProfile {
  soul: { revision: string; personality: Personality };
  traits: PersonalityChoice[];
  behaviors: PersonalityChoice[];
  preview: string;
  memories: MemoryEntry[];
  memoryEnabled: boolean;
  personalityEnabled: boolean;
}
export type ProfileChange =
  | { type: "personality"; expected_revision: string; personality: Personality }
  | { type: "add_memory"; text: string }
  | { type: "edit_memory"; expected: MemoryEntry; text: string }
  | { type: "delete_memory"; expected: MemoryEntry }
  | { type: "clear_memories"; expected: MemoryEntry[] };
export const loadAgentProfile = () => forOwner(() => invoke<AgentProfile>("load_agent_profile"), "Studio couldn't load Orion's personality and memories. Please try again.");
export const changeAgentProfile = (change: ProfileChange) => forOwner(() => invoke<AgentProfile>("change_agent_profile", { change }), "Studio couldn't save these changes. Your draft is preserved; please try again.");
export function toggleSelection(personality: Personality, group: keyof Personality, id: string): Personality {
  return { ...personality, [group]: personality[group].includes(id) ? personality[group].filter(value => value !== id) : [...personality[group],id] };
}
export function samePersonality(a: Personality,b: Personality) {
  return (["traits","behaviors"] as const).every(group => a[group].length === b[group].length && a[group].every(id => b[group].includes(id)));
}
export function validMemory(text: string) {
  return !!text.trim() && new TextEncoder().encode(text.trim()).length <= 2000 && !text.includes("<!--") && !text.includes("-->") && !text.includes("\0");
}
