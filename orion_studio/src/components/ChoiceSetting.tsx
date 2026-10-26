import { useId } from "react";
import "./ChoiceSetting.css";

/** A short set of choices in one row. Only the chosen one is explained. */
export function ChoiceSetting<T extends string>({ label, options, value, disabled, onChange }: {
  label: string; options: [T, string, string][]; value?: T; disabled?: boolean; onChange: (value: T) => void;
}) {
  const id = useId(), chosen = options.find(([option]) => option === value);
  return <div className="choice-setting" role="radiogroup" aria-labelledby={id} aria-disabled={disabled || value === undefined}>
    <strong id={id}>{label}</strong>
    <div className="choice-segments">{options.map(([option, name]) => <label key={option} className={option === value ? "chosen" : ""}>
      <input type="radio" name={id} value={option} checked={option === value} disabled={disabled || value === undefined}
        onChange={() => onChange(option)}/><span>{name}</span></label>)}</div>
    {chosen && <p>{chosen[2]}</p>}
  </div>;
}
