import { useEffect, useRef, useState } from "react";
import { normalizeReciterSearch } from "./audioCatalog";

type Option = { id: number; name: string };
/** Both listening modes share keyboard and pointer selection behavior. */
export function ReciterPicker({ id, label, prompt, empty, query, onQuery, selected, options, onSelect }: {
  id: string; label: string; prompt: string; empty: string; query: string;
  onQuery: (query: string) => void; selected: number; options: Option[]; onSelect: (id: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const list = useRef<HTMLDivElement>(null);
  const ranked = [...options].sort((a, b) =>
    Number(normalizeReciterSearch(b.name) === normalizeReciterSearch(query)) -
    Number(normalizeReciterSearch(a.name) === normalizeReciterSearch(query)));
  const choose = (option: Option) => { onSelect(option.id); setOpen(false); setActive(-1); };
  useEffect(() => { list.current?.children[active]?.scrollIntoView({ block: "nearest" }); }, [active]);
  return <div className="reciterCombobox" onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) { setOpen(false); setActive(-1); }
  }}>
    <label htmlFor={`${id}-search`}>{label}</label>
    <input id={`${id}-search`} type="search" role="combobox" aria-autocomplete="list"
      aria-controls={`${id}-options`} aria-expanded={open} aria-label={prompt} placeholder={prompt}
      aria-activedescendant={open && ranked[active] ? `${id}-option-${ranked[active].id}` : undefined}
      value={query} onFocus={event => { setOpen(true); setActive(-1); event.currentTarget.select(); }}
      onChange={event => { onQuery(event.target.value); setOpen(true); setActive(-1); }}
      onKeyDown={event => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault(); setOpen(true);
          setActive(index => ranked.length ? (event.key === "ArrowDown"
            ? Math.min(ranked.length - 1, index + 1) : index < 0 ? ranked.length - 1 : Math.max(0, index - 1)) : -1);
        } else if (event.key === "Enter" && open && ranked.length) {
          event.preventDefault(); choose(ranked[active < 0 ? 0 : active]);
        } else if (event.key === "Escape" && open) {
          event.preventDefault(); event.stopPropagation(); setOpen(false); setActive(-1);
        }
      }} />
    {open ? <div ref={list} id={`${id}-options`} className="reciterOptions" role="listbox" aria-label={label}>
      {ranked.map((option, index) => <button key={option.id} id={`${id}-option-${option.id}`} type="button"
        role="option" aria-selected={option.id === selected} className={active === index ? "reciterOptionActive" : undefined}
        onClick={() => choose(option)}>{option.name}</button>)}
      {ranked.length === 0 ? <p role="status">{empty}</p> : null}
    </div> : null}
  </div>;
}
