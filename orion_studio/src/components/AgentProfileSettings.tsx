import { useEffect, useId, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { House, NotebookPen, Sparkles } from "lucide-react";
import { changeAgentProfile, loadAgentProfile, samePersonality, toggleSelection, validMemory, type AgentProfile, type MemoryEntry, type Personality, type ProfileChange } from "../lib/agentProfile";
import "./AgentProfileSettings.css";
import { failure, type Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";
import { useScopedNotice } from "../hooks/useScopedNotice";

export const PROFILE_SAVING_NOTICE = "Saving… If Orion is replying, this will apply when it finishes.";

export function AgentProfileSettings({ onboard = false, connectionScope, connected = true, desktop = isTauri() }: { onboard?: boolean; connectionScope?: unknown; connected?: boolean; desktop?: boolean }) {
  const available = desktop && connected;
  const [profile,setProfile] = useState<AgentProfile|null>(null);
  const [personality,setPersonality] = useState<Personality>({traits:[],behaviors:[]});
  const [loading,setLoading] = useState(true);
  const [pending,setPending] = useState(false);
  const [error,setError] = useState<Failure | null>(null);
  const [notice,setNotice] = useScopedNotice("profile", connectionScope);
  const [query,setQuery] = useState("");
  const [editor,setEditor] = useState<MemoryEntry|"new"|null>(null);
  const [text,setText] = useState("");
  const [deletion,setDeletion] = useState<MemoryEntry|"all"|null>(null);
  const [forgetHome,setForgetHome] = useState(false);
  const labelId = useId();
  const operation = useRef(false);
  const generation = useRef(0);
  const dirty = profile !== null && !samePersonality(personality,profile.soul.personality);
  const refresh = async () => {
    const epoch = generation.current;
    setLoading(true); setError(null); setNotice("");
    try { const next = await loadAgentProfile(); if (epoch !== generation.current) return; setProfile(next); setPersonality(next.soul.personality); }
    catch(error) { if (epoch === generation.current) setError(failure("Studio couldn't load Orion's personality and memories. Please try again.", error)); }
    finally { if (epoch === generation.current) setLoading(false); }
  };
  useEffect(() => {
    if (!available) return;
    generation.current++;
    setProfile(null); setError(null); setLoading(true); setEditor(null); setDeletion(null); setQuery("");
    let active = true;
    void loadAgentProfile().then(next => { if (active) { setProfile(next); setPersonality(next.soul.personality); } }).catch(error => { if (active) setError(failure("Studio couldn't load Orion's personality and memories. Please try again.", error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; generation.current++; };
  },[available,connectionScope]);
  const save = async (change: ProfileChange) => {
    if (operation.current) return;
    operation.current = true;
    const epoch = generation.current;
    setPending(true); setError(null); setNotice("");
    try {
      const next = await changeAgentProfile(change);
      if (epoch !== generation.current) return;
      setProfile(next);
      // Keep choices made during the save; the next autosave uses the returned revision.
      if (change.type === "personality") setPersonality(current => samePersonality(current,change.personality) ? next.soul.personality : current);
      if (change.type !== "personality") { setEditor(null); setDeletion(null); setForgetHome(false); }
      setNotice("Saved. Orion’s next request starts a fresh conversation with these changes.");
    } catch(error) { if (epoch === generation.current) setError(failure("Studio couldn't save these changes. Your draft is preserved; please try again.", error)); }
    finally { operation.current = false; setPending(false); }
  };
  useEffect(() => {
    if (!available || !profile?.personalityEnabled || !dirty || pending || error) return;
    const timer = setTimeout(() => void save({type:"personality",expected_revision:profile.soul.revision,personality}),300);
    return () => clearTimeout(timer);
  },[available,profile,personality,dirty,pending,error]);
  const choose = (group: keyof Personality, id: string) => { setError(null); setNotice(""); setPersonality(old => toggleSelection(old,group,id)); };
  const commitMemory = () => {
    if (editor === null || pending || !validMemory(text)) return;
    if (editor !== "new" && text.trim() === editor.text) { setEditor(null); return; }
    void save(editor === "new" ? {type:"add_memory",text:text.trim()} : {type:"edit_memory",expected:editor,text:text.trim()});
  };
  const beginEdit = (entry: MemoryEntry|"new") => { setEditor(entry); setText(entry === "new" ? "" : entry.text); setDeletion(null); setNotice(""); setError(null); };
  const blocked = loading || pending;
  const visible = profile?.memories.filter(entry => entry.text.toLocaleLowerCase().includes(query.toLocaleLowerCase())) ?? [];
  const preview = profile ? [...profile.traits.filter(choice => personality.traits.includes(choice.id)),...profile.behaviors.filter(choice => personality.behaviors.includes(choice.id))] : [];
  const editorForm = editor !== null && <div className="memory-editor">
    <label htmlFor={`${labelId}-memory`}>{editor === "new" ? "What should Orion remember?" : "Edit memory"}</label>
    <textarea id={`${labelId}-memory`} value={text} rows={3} maxLength={2000} autoFocus readOnly={pending} onChange={event => { setText(event.target.value); setError(null); }} onBlur={event => { if (!(event.relatedTarget instanceof HTMLElement && event.relatedTarget.dataset.cancelMemory)) commitMemory(); }} placeholder="For example: I prefer temperatures in Celsius." />
    <p className="settings-help">Valid text saves automatically when you leave this field.</p>
    {text.length > 0 && !validMemory(text) && <p className="settings-error">Use a short, non-empty memory without reserved comment markers.</p>}
    <div className="profile-actions"><button className="quiet-button" type="button" disabled={pending || !validMemory(text)} onClick={commitMemory}>Done</button><button className="quiet-button" type="button" data-cancel-memory="true" disabled={pending} onClick={() => setEditor(null)}>Cancel</button></div>
  </div>;
  if (!available) return <section className="agent-profile" aria-label="Personality and memories"><p className="settings-help">{desktop ? "Connect Orion to view or change personality and memories." : "Open Orion Studio’s desktop app and connect Orion to view or change personality and memories."}</p></section>;
  return <section className="agent-profile" aria-label="Personality and memories">
    <div className="profile-status" aria-live="polite">{loading && <p>Loading personality and memories…</p>}{pending && <p>{PROFILE_SAVING_NOTICE}</p>}{notice && <p>{notice}</p>}</div>
    {error && <div className="profile-error"><FailureNotice value={error} />{!profile && <button className="quiet-button" disabled={loading} onClick={() => void refresh()}>Try again</button>}{profile && <><p>Your draft is preserved.</p>{(dirty || editor) && <button className="quiet-button" disabled={pending} onClick={() => { if (editor) commitMemory(); else void save({type:"personality",expected_revision:profile.soul.revision,personality}); }}>Try again</button>}</>}</div>}
    {profile?.reminderError && <div className="profile-error"><FailureNotice value={{ message: "Orion couldn't read next-conversation reminders. Replies will continue without them.", detail: profile.reminderError }} /><button className="quiet-button" disabled={loading || pending} onClick={() => void refresh()}>Refresh</button></div>}
    {profile && <div className="settings-stack">
      <section className="profile-subsection personality-settings"><header><Sparkles size={20}/><div><h3>How Orion talks</h3><p>Choose how Orion sounds and responds.</p></div></header>
        <p className="settings-help">Mix the traits that feel right. Tool permissions and privacy rules always stay in place.</p>
        {profile && <>
          <fieldset disabled={loading || !profile.personalityEnabled}><legend>Personality traits</legend><div className="personality-traits">{profile.traits.map(choice => <label key={choice.id} className={`personality-trait ${personality.traits.includes(choice.id) ? "selected" : ""}`}><input type="checkbox" checked={personality.traits.includes(choice.id)} onChange={() => choose("traits",choice.id)}/><span>{choice.label}</span></label>)}</div></fieldset>
          <fieldset disabled={loading || !profile.personalityEnabled}><legend>Conversational habits</legend>{profile.behaviors.map(choice => <label className="personality-behavior" key={choice.id}><input type="checkbox" checked={personality.behaviors.includes(choice.id)} onChange={() => choose("behaviors",choice.id)}/><span><strong>{choice.label}</strong><small>{choice.description}</small></span></label>)}</fieldset>
          <details className="personality-preview"><summary>Preview Orion’s personality</summary>{preview.length ? <ul>{preview.map(choice => <li key={choice.id}>{choice.description}</li>)}</ul> : <p>Use a neutral, clear conversational tone.</p>}</details>
          {!profile.personalityEnabled && <p className="settings-help">Personality storage is unavailable on this device.</p>}
          <p className="settings-help">Choices save automatically. {dirty ? error ? "Unsaved changes" : "Waiting to save…" : "Saved"}</p>
          {dirty && error && <button className="quiet-button" disabled={blocked} onClick={() => { setPersonality(profile.soul.personality); setError(null); }}>Discard changes</button>}
        </>}
      </section>
      <section className="profile-subsection memory-settings"><header><NotebookPen size={20}/><div><h3>Memories</h3><p>Saved facts and preferences. Only exceptionally important, lasting facts are extracted from retired conversations.</p></div></header>
        <p className="settings-help">{onboard ? "Stored on Orion." : "Stored on this computer."} Relevant memories are sent to Codex when Orion uses them. Retired conversations are reviewed by Codex for important lasting facts. Editing or deleting refreshes agent context; it does not erase information already sent to Codex.</p>
        {profile && <>
          <div className="memory-toolbar"><button className="quiet-button" disabled={blocked || !!editor || !!deletion || !profile.memoryEnabled} onClick={() => beginEdit("new")}>Add memory</button><button className="quiet-button" disabled={blocked || !!editor || !!deletion || dirty} onClick={() => void refresh()}>Refresh</button><span>{profile.memories.length} {profile.memories.length === 1 ? "memory" : "memories"}</span></div>
          {!profile.memoryEnabled && <p className="settings-help">Memory storage is unavailable on this device.</p>}
          {profile.memories.length > 0 && <label className="memory-search">Search memories<input type="search" disabled={!!editor || !!deletion} value={query} onChange={event => setQuery(event.target.value)} placeholder="Find a fact or preference" /></label>}
          {editor === "new" && editorForm}
          {!profile.memories.length && editor !== "new" && <div className="memory-empty"><p>No memories yet.</p><small>Say “Remember that I prefer Celsius”, or add a memory here.</small></div>}
          {profile.memories.length > 0 && !visible.length && <p className="settings-help">No memories match your search.</p>}
          <ul className="memory-list">{visible.map(entry => <li key={entry.id}>
            {editor !== null && editor !== "new" && editor.id === entry.id ? editorForm : <><p className="memory-text">{entry.text}</p><time dateTime={entry.created}>Saved {new Date(entry.created).toLocaleDateString(undefined,{year:"numeric",month:"short",day:"numeric"})}</time>{entry.source && <p className="settings-help">Extracted from the conversation starting {new Date(entry.source.conversationStartedAt).toLocaleString()}.<span title={`Conversation ${entry.source.conversationId}; turns ${entry.source.turnIds.join(", ")}`}> Source recorded.</span></p>}<div className="profile-actions"><button className="quiet-button" disabled={blocked || !!editor || !!deletion} onClick={() => beginEdit(entry)}>Edit<span className="profile-sr-only"> memory: {entry.text}</span></button><button className="quiet-button memory-delete" disabled={blocked || !!editor || !!deletion} onClick={() => setDeletion(entry)}>Delete<span className="profile-sr-only"> memory: {entry.text}</span></button></div></>}
          </li>)}</ul>
          {profile.memories.length > 0 && <button className="quiet-button memory-delete" disabled={blocked || !!editor || !!deletion} onClick={() => setDeletion("all")}>Clear all memories</button>}
          {deletion && <div className="memory-confirm" role="group" aria-label="Confirm memory deletion"><p>{deletion === "all" ? `Delete all ${profile.memories.length} memories?` : "Delete this memory?"}</p>{deletion !== "all" && <blockquote>{deletion.text}</blockquote>}<p className="settings-help">This cannot be undone.</p><div className="profile-actions"><button className="quiet-button memory-delete" disabled={pending} onClick={() => void save(deletion === "all" ? {type:"clear_memories",expected:profile.memories} : {type:"delete_memory",expected:deletion})}>Confirm deletion</button><button className="quiet-button" disabled={pending} onClick={() => setDeletion(null)}>Cancel</button></div></div>}
        </>}
      </section>
      <HomeMemorySection profile={profile} blocked={blocked || !!editor || !!deletion} confirming={forgetHome} onConfirming={setForgetHome} onChange={change => void save({type:"home",change})} />
    </div>}
  </section>;
}

const day = (value: string) => new Date(`${value}T12:00:00`).toLocaleDateString(undefined,{weekday:"long",month:"short",day:"numeric"});
/** What Orion learned about its home by itself. */
export function HomeMemorySection({ profile, blocked, confirming, onConfirming, onChange }: { profile: AgentProfile; blocked: boolean; confirming: boolean; onConfirming: (value: boolean) => void; onChange: (change: import("../lib/agentProfile").HomeChange) => void }) {
  const followUps = profile.home?.followUps.filter(item => !item.done) ?? [];
  const days = [...profile.home?.days ?? []].reverse();
  const habits = profile.habits ?? [];
  const empty = !followUps.length && !days.length && !habits.length;
  return <section className="profile-subsection home-memory"><header><House size={20}/><div><h3>Orion’s home memory</h3><p>What Orion noticed and learned by itself. It stays on Orion. Only the habits and the last few days are sent to Codex, when you talk to Orion.</p></div></header>
    <h4>Things to come back to</h4>
    {followUps.length ? <ul className="memory-list">{followUps.map(item => <li key={item.id}><p className="memory-text">{item.text}</p><time dateTime={item.on}>Orion asks about it on {day(item.on)}, only when you talk to it</time>
      <div className="profile-actions"><button className="quiet-button memory-delete" disabled={blocked} onClick={() => onChange({type:"delete_follow_up",id:item.id})}>Delete<span className="profile-sr-only">: {item.text}</span></button></div></li>)}</ul>
      : <p className="settings-help">Nothing yet. Tell Orion about an interview or a trip, and it will ask how it went.</p>}
    <h4>Habits</h4>
    {habits.length ? <><ul className="home-habits">{habits.map(text => <li key={text}>{text}</li>)}</ul><button className="quiet-button memory-delete" disabled={blocked} onClick={() => onChange({type:"clear_habits"})}>Forget habits</button></>
      : <p className="settings-help">Orion is still learning your habits. They appear after a few days.</p>}
    <h4>Recent days</h4>
    {days.length ? <ul className="memory-list">{days.map(item => <li key={item.day}><p className="memory-text">{item.text}</p>
      <div className="profile-actions"><button className="quiet-button memory-delete" disabled={blocked} onClick={() => onChange({type:"delete_day",day:item.day})}>Delete<span className="profile-sr-only"> {day(item.day)}</span></button></div></li>)}</ul>
      : <p className="settings-help">Orion writes one short line about each day, the morning after.</p>}
    {!empty && !confirming && <button className="quiet-button memory-delete" disabled={blocked} onClick={() => onConfirming(true)}>Forget everything about home</button>}
    {confirming && <div className="memory-confirm" role="group" aria-label="Confirm forgetting home memory"><p>Forget everything Orion learned about its home?</p><p className="settings-help">Saved memories above stay. This cannot be undone.</p><div className="profile-actions"><button className="quiet-button memory-delete" disabled={blocked} onClick={() => onChange({type:"clear_all"})}>Forget</button><button className="quiet-button" onClick={() => onConfirming(false)}>Cancel</button></div></div>}
  </section>;
}
