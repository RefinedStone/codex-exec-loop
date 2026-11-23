import { SkillsPanel } from "../skills/SkillsPanel";
import type { RegisteredPanelProps } from "./registry";
export default function SkillsRegisteredPanel({ skills }: RegisteredPanelProps) {
  return <SkillsPanel {...skills} />;
}
