import { useEffect, useId, useState } from "react";
import { NotebookPen, Sparkles } from "lucide-react";
import { changeAgentProfile, loadAgentProfile, samePersonality, toggleSelection, validMemory, type AgentProfile, type MemoryEntry, type Personality, type ProfileChange } from "../lib/agentProfile";
import "./AgentProfileSettings.css";

export function AgentProfileSettings() {
  const [profile,setProfile] = useState<AgentProfile|null>(null);
  const [personality,setPersonality] = useState<Personality>({traits:[],behaviors:[]});
  const [loading,setLoading] = useState(true);
  const [pending,setPending] = useState(false);
  const [error,setError] = useState("");
  const [notice,setNotice] = useState("");
  const [query,setQuery] = useState("");
  const [editor,setEditor] = useState<MemoryEntry|"new"|null>(null);
  const [text,setText] = useState("");
  const [deletion,setDeletion] = useState<MemoryEntry|"all"|null>(null);
  const labelId = useId();
  const dirty = profile !== null && !samePersonality(personality,profile.soul.personality);
  const refresh = async () => {
    setLoading(true); setError(""); setNotice("");
    try { const next = await loadAgentProfile(); setProfile(next); setPersonality(next.soul.personality); }
    catch(error) { setError(String(error)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    let active = true;
    void loadAgentProfile().then(next => { if (active) { setProfile(next); setPersonality(next.soul.personality); } }).catch(error => { if (active) setError(String(error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  },[]);
  const save = async (change: ProfileChange) => {
    setPending(true); setError(""); setNotice("");
    try {
      const next = await changeAgentProfile(change);
      setProfile(change.type !== "personality" && dirty && profile ? { ...next, soul: profile.soul } : next);
      if (change.type === "personality" || !dirty) setPersonality(next.soul.personality);
      if (change.type !== "personality") { setEditor(null); setDeletion(null); }
      setNotice("Saved. Orion’s next request starts a fresh conversation with these changes.");
    } catch(error) { setError(String(error)); }
    finally { setPending(false); }
  };
  const beginEdit = (entry: MemoryEntry|"new") => { setEditor(entry); setText(entry === "new" ? "" : entry.text); setDeletion(null); setNotice(""); setError(""); };
  const blocked = loading || pending;
  const visible = profile?.memories.filter(entry => entry.text.toLocaleLowerCase().includes(query.toLocaleLowerCase())) ?? [];
  const preview = profile ? [...profile.traits.filter(choice => personality.traits.includes(choice.id)),...profile.behaviors.filter(choice => personality.behaviors.includes(choice.id))] : [];
  const editorForm = editor !== null && <form className="memory-editor" onSubmit={event => { event.preventDefault(); if (!pending && validMemory(text)) void save(editor === "new" ? {type:"add_memory",text:text.trim()} : {type:"edit_memory",expected:editor,text:text.trim()}); }}>
    <label htmlFor={`${labelId}-memory`}>{editor === "new" ? "What should Orion remember?" : "Edit memory"}</label>
    <textarea id={`${labelId}-memory`} value={text} rows={3} maxLength={2000} autoFocus disabled={pending} onChange={event => setText(event.target.value)} placeholder="For example: I prefer temperatures in Celsius." />
    {text.length > 0 && !validMemory(text) && <p className="settings-error">Use a short, non-empty memory without reserved comment markers.</p>}
    <div className="profile-actions"><button className="primary-button" type="submit" disabled={pending || !validMemory(text)}>{pending ? "Saving…" : "Save memory"}</button><button className="quiet-button" type="button" disabled={pending} onClick={() => setEditor(null)}>Cancel</button></div>
  </form>;
  return <section className="agent-profile" aria-label="Personality and memories">
    <div className="profile-status" aria-live="polite">{loading && <p>Loading personality and memories…</p>}{pending && <p>Saving… If Orion is replying, this will apply when she finishes.</p>}{notice && <p>{notice}</p>}</div>
    {error && <div role="alert" className="profile-error"><p>{error}</p>{!profile && <button className="quiet-button" disabled={loading} onClick={() => void refresh()}>Try again</button>}{profile && <p>Your draft is preserved. Cancel or discard changes before refreshing.</p>}</div>}
    <div className="settings-columns">
      <section className="settings-card personality-settings"><header><Sparkles size={20}/><div><h2>Personality</h2><p>Choose how Orion sounds and responds.</p></div></header>
        <p className="settings-help">Mix the traits that feel right. Tool permissions and privacy rules always stay in place.</p>
        {profile && <>
          <fieldset disabled={blocked || !profile.personalityEnabled}><legend>Personality traits</legend><div className="personality-traits">{profile.traits.map(choice => <label key={choice.id} className={`personality-trait ${personality.traits.includes(choice.id) ? "selected" : ""}`}><input type="checkbox" checked={personality.traits.includes(choice.id)} onChange={() => setPersonality(old => toggleSelection(old,"traits",choice.id))}/><span>{choice.label}</span></label>)}</div></fieldset>
          <fieldset disabled={blocked || !profile.personalityEnabled}><legend>Conversational habits</legend>{profile.behaviors.map(choice => <label className="personality-behavior" key={choice.id}><input type="checkbox" checked={personality.behaviors.includes(choice.id)} onChange={() => setPersonality(old => toggleSelection(old,"behaviors",choice.id))}/><span><strong>{choice.label}</strong><small>{choice.description}</small></span></label>)}</fieldset>
          <details className="personality-preview"><summary>Preview Orion’s personality</summary>{preview.length ? <ul>{preview.map(choice => <li key={choice.id}>{choice.description}</li>)}</ul> : <p>Use a neutral, clear conversational tone.</p>}</details>
          {!profile.personalityEnabled && <p className="settings-help">Personality storage is unavailable on this device.</p>}
          <div className="profile-actions"><button className="primary-button" disabled={blocked || !dirty || !profile.personalityEnabled} onClick={() => void save({type:"personality",expected_revision:profile.soul.revision,personality})}>Save personality</button>{dirty && <button className="quiet-button" disabled={blocked} onClick={() => setPersonality(profile.soul.personality)}>Discard changes</button>}</div>
        </>}
      </section>
      <section className="settings-card memory-settings"><header><NotebookPen size={20}/><div><h2>Memories</h2><p>Facts and preferences you’ve asked Orion to remember.</p></div></header>
        <p className="settings-help">Stored on this computer. Relevant memories are sent to Codex when Orion uses them. Editing or deleting starts a fresh conversation; it does not erase information already sent to Codex.</p>
        {profile && <>
          <div className="memory-toolbar"><button className="quiet-button" disabled={blocked || !!editor || !!deletion || !profile.memoryEnabled} onClick={() => beginEdit("new")}>Add memory</button><button className="quiet-button" disabled={blocked || !!editor || !!deletion || dirty} onClick={() => void refresh()}>Refresh</button><span>{profile.memories.length} {profile.memories.length === 1 ? "memory" : "memories"}</span></div>
          {!profile.memoryEnabled && <p className="settings-help">Memory storage is unavailable on this device.</p>}
          {profile.memories.length > 0 && <label className="memory-search">Search memories<input type="search" disabled={!!editor || !!deletion} value={query} onChange={event => setQuery(event.target.value)} placeholder="Find a fact or preference" /></label>}
          {editor === "new" && editorForm}
          {!profile.memories.length && editor !== "new" && <div className="memory-empty"><p>No memories yet.</p><small>Say “Remember that I prefer Celsius”, or add a memory here.</small></div>}
          {profile.memories.length > 0 && !visible.length && <p className="settings-help">No memories match your search.</p>}
          <ul className="memory-list">{visible.map(entry => <li key={entry.id}>
            {editor !== null && editor !== "new" && editor.id === entry.id ? editorForm : <><p className="memory-text">{entry.text}</p><time dateTime={entry.created}>Saved {new Date(entry.created).toLocaleDateString(undefined,{year:"numeric",month:"short",day:"numeric"})}</time><div className="profile-actions"><button className="quiet-button" disabled={blocked || !!editor || !!deletion} onClick={() => beginEdit(entry)}>Edit<span className="profile-sr-only"> memory: {entry.text}</span></button><button className="quiet-button memory-delete" disabled={blocked || !!editor || !!deletion} onClick={() => setDeletion(entry)}>Delete<span className="profile-sr-only"> memory: {entry.text}</span></button></div></>}
          </li>)}</ul>
          {profile.memories.length > 0 && <button className="quiet-button memory-delete" disabled={blocked || !!editor || !!deletion} onClick={() => setDeletion("all")}>Clear all memories</button>}
          {deletion && <div className="memory-confirm" role="group" aria-label="Confirm memory deletion"><p>{deletion === "all" ? `Delete all ${profile.memories.length} memories?` : "Delete this memory?"}</p>{deletion !== "all" && <blockquote>{deletion.text}</blockquote>}<p className="settings-help">This cannot be undone.</p><div className="profile-actions"><button className="quiet-button memory-delete" disabled={pending} onClick={() => void save(deletion === "all" ? {type:"clear_memories",expected:profile.memories} : {type:"delete_memory",expected:deletion})}>Confirm deletion</button><button className="quiet-button" disabled={pending} onClick={() => setDeletion(null)}>Cancel</button></div></div>}
        </>}
      </section>
    </div>
  </section>;
}
