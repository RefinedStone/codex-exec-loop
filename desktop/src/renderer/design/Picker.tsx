// 값 하나를 고르는 작은 메뉴이다. 실제 앱과 UX 블루프린트가 함께 쓴다.
import { useState } from "react";

import { Icon } from "./icons";

export function Picker({ icon, value, options, onChange }: { icon?: React.ReactNode; value: string; options: readonly string[]; onChange(value: string): void }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="picker">
      <button type="button" className="chip-button" onClick={() => setOpen(!open)} aria-expanded={open}>
        {icon}
        {value}
        <Icon name="chevronDown" size={11} />
      </button>
      {open && (
        <>
          <div className="backdrop" onClick={() => setOpen(false)} />
          <ul className="menu" role="listbox">
            {options.map((option) => (
              <li key={option}>
                <button
                  type="button"
                  role="option"
                  aria-selected={option === value}
                  onClick={() => {
                    onChange(option);
                    setOpen(false);
                  }}
                >
                  <span className="menu-check">{option === value && <Icon name="check" size={12} />}</span>
                  {option}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </span>
  );
}
