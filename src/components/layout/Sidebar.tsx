// src/components/layout/Sidebar.tsx
import { NavLink } from "react-router";
import {
  FolderGit2,
  FileCode2,
  ArrowRightLeft,
  FileText,
  Cable,
  Eye,
  Settings,
  Terminal,
} from "lucide-react";
import { cn } from "@/lib/utils";

const primaryNav = [
  { to: "/repos", label: "Repos", icon: FolderGit2 },
  { to: "/recipes", label: "Recipes", icon: FileCode2 },
  { to: "/transforms", label: "Transforms", icon: ArrowRightLeft },
  { to: "/templates", label: "Templates", icon: FileText },
  { to: "/bindings", label: "Bindings", icon: Cable },
  { to: "/watch", label: "Watch", icon: Eye },
];

const secondaryNav = [
  { to: "/settings", label: "Settings", icon: Settings },
  { to: "/dev-tools", label: "Dev Tools", icon: Terminal },
];

function NavItem({
  to,
  label,
  icon: Icon,
}: {
  to: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-2 px-3 py-2 text-sm rounded-md transition-colors",
          isActive
            ? "bg-bg text-accent border-l-2 border-accent"
            : "text-text-muted hover:text-text hover:bg-bg/50"
        )
      }
    >
      <Icon className="h-4 w-4 shrink-0" />
      {label}
    </NavLink>
  );
}

export function Sidebar() {
  return (
    <aside className="w-[220px] shrink-0 bg-surface border-r border-border flex flex-col h-full">
      {/* Title */}
      <div className="px-4 py-4">
        <h1 className="text-sm font-semibold text-text">Flatten PM</h1>
      </div>

      {/* Primary nav */}
      <nav className="flex flex-col gap-0.5 px-2">
        {primaryNav.map((item) => (
          <NavItem key={item.to} {...item} />
        ))}
      </nav>

      <hr className="border-border-muted my-2 mx-2" />

      {/* Secondary nav */}
      <nav className="flex flex-col gap-0.5 px-2">
        {secondaryNav.map((item) => (
          <NavItem key={item.to} {...item} />
        ))}
      </nav>

      {/* Bottom section: watch status + theme toggle */}
      <div className="mt-auto px-3 py-3 border-t border-border-muted">
        {/* MOCK: replace with Tauri command */}
        <div className="flex items-center gap-2 text-xs text-text-muted">
          <span className="h-2 w-2 rounded-full bg-success shrink-0" />
          <span>Idle</span>
          <span className="ml-auto">0 flags</span>
        </div>
        {/* ThemeToggle: C6 */}
      </div>
    </aside>
  );
}
