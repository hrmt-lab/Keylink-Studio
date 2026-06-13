interface Props {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  /** Accessible label describing what the toggle controls. */
  label?: string;
}

// Knob geometry: 14px knob inside a 36px-wide groove with 3px margins.
const KNOB_LEFT_OFF = 3;
const KNOB_LEFT_ON = 19;

export function Toggle({ checked, onChange, disabled = false, label }: Props) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`group relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors duration-200 ${
        checked ? "bg-accent shadow-neu-toggle-on" : "bg-plate shadow-neu-groove"
      } ${disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer"}`}
    >
      <span
        className="toggle-knob absolute h-3.5 w-3.5 rounded-full bg-white shadow-neu-knob group-active:w-[17px]"
        style={{ left: checked ? KNOB_LEFT_ON : KNOB_LEFT_OFF }}
      />
    </button>
  );
}
