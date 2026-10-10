import { renderToStaticMarkup } from "react-dom/server";
import { it, expect, vi } from "vitest";
import { ListRows, ListsCard, listText } from "./ListsCard";
import { agentLists } from "../lib/gateway";
it("renders readable item checkboxes and preserves plain text for copying", () => {
 const items = [{id:"a",text:"Milk",created:"2026-01-01T00:00:00Z"},{id:"b",text:"Bread",created:"2026-01-01T00:00:00Z"}];
 const html=renderToStaticMarkup(<ListRows items={items} disabled={false} onRemove={vi.fn()} />);
 expect(html.match(/type="checkbox"/g)).toHaveLength(2);expect(html).toContain("Milk");expect(listText(items)).toBe("Milk\nBread");
 expect(renderToStaticMarkup(<ListsCard connection={{url:"http://pi",token:"x"}} />)).toContain('aria-label="Shopping and to-do lists"');
});
it("routes edits through the authenticated private service with the snapshot", async () => {
 const fetcher=vi.fn().mockResolvedValue({ok:true,json:async()=>[]});vi.stubGlobal("fetch",fetcher);
 const expected=[{id:"a",text:"Milk",created:"2026-01-01T00:00:00Z"}];
 await agentLists({url:"http://pi",token:"token"},"shopping",{type:"clear",expected,confirmed:true});
 expect(fetcher.mock.calls[0][0]).toBe("http://pi/api/v2/voice/request");
 expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({method:"lists",params:{list:"shopping",change:{type:"clear",expected,confirmed:true}}});
 vi.unstubAllGlobals();
});
