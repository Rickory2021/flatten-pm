// src/pages/SettingsPage.tsx
import { Settings } from "lucide-react";

export function SettingsPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <Settings className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Settings</h1>
      </div>
      <p className="text-text-muted mt-1">
        Global application settings.
      </p>
    </div>
  );
}
