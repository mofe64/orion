import { useEffect, useRef, useState } from "react";
import { agentLists, type GatewayConnection, type ListItem, type ListName, type ListChange } from "../lib/gateway";
export function listText(items: ListItem[]): string { return items.map(item => item.text).join("\n"); }
export function ListRows({ items, disabled, onRemove }: { items: ListItem[]; disabled: boolean; onRemove: (item: ListItem) => void }) {
  return <ul>{items.map(item => <li key={item.id}><label><input type="checkbox" checked={false} disabled={disabled} onChange={() => onRemove(item)} />{item.text}</label></li>)}</ul>;
}
export function ListsCard({ connection }: { connection: GatewayConnection }) {
  const [list, setList] = useState<ListName>("shopping"); const [items, setItems] = useState<ListItem[]>([]);
  const [text, setText] = useState(""); const [pending, setPending] = useState(false); const [loading, setLoading] = useState(true);
  const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  const [clear, setClear] = useState<ListItem[] | null>(null); const epoch = useRef(0);
  useEffect(() => {
    const generation = ++epoch.current; setItems([]); setText(""); setClear(null); setLoading(true); setError(""); setNotice("");
    void agentLists(connection, list).then(value => { if (generation === epoch.current) setItems(value); })
      .catch(value => { if (generation === epoch.current) setError(String(value instanceof Error ? value.message : value)); })
      .finally(() => { if (generation === epoch.current) setLoading(false); });
    return () => { epoch.current++; };
  }, [connection, list]);
  const change = async (value?: ListChange) => {
    if (pending) return; const generation = epoch.current; setPending(true); setError(""); setNotice("");
    try { const result = await agentLists(connection, list, value); if (generation !== epoch.current) return; setItems(result); setClear(null); if (value?.type === "add") setText(""); }
    catch (value) { if (generation === epoch.current) setError(value instanceof Error ? value.message : String(value)); }
    finally { setPending(false); }
  };
  const disabled = pending || loading;
  return <section className="oh-alerts" aria-label="Shopping and to-do lists">
    <header className="oh-panel-heading"><h2>Lists</h2><select aria-label="List" value={list} disabled={pending} onChange={event => setList(event.target.value as ListName)}><option value="shopping">Shopping</option><option value="todo">To-do</option></select></header>
    {loading ? <p role="status">Loading list…</p> : items.length ? <ListRows items={items} disabled={disabled} onRemove={item => void change({type:"remove",expected:item})} /> : <p>Your list is empty.</p>}
    <form className="oh-reminder-form" onSubmit={event => { event.preventDefault(); if (text.trim()) void change({type:"add",text:text.trim()}); }}><label>New item<input required maxLength={200} value={text} disabled={disabled} onChange={event => setText(event.target.value)} /></label><button disabled={disabled || !text.trim()}>Add item</button></form>
    <div className="profile-actions"><button className="quiet-button" disabled={disabled} onClick={() => void change()}>Refresh list</button><button className="quiet-button" disabled={disabled || !items.length} onClick={() => {
      const generation=epoch.current;
      void navigator.clipboard.writeText(listText(items)).then(()=> {if(generation===epoch.current)setNotice("List copied.");}).catch(()=>{if(generation===epoch.current)setError("Couldn’t copy the list. Please try again.");});
    }}>Copy list</button><button className="quiet-button" disabled={disabled || !items.length} onClick={() => setClear([...items])}>Clear list</button></div>
    {clear && <div role="group" aria-label="Confirm clear list"><p>Clear all {clear.length} items from your {list === "todo" ? "to-do" : "shopping"} list?</p><button disabled={pending} onClick={() => void change({type:"clear",expected:clear,confirmed:true})}>Yes, clear list</button><button disabled={pending} onClick={() => setClear(null)}>Keep items</button></div>}
    {error && <p role="alert">{error}</p>}{notice && <p role="status">{notice}</p>}
  </section>;
}
