// src/components/layout/Layout.tsx
import { Outlet } from "react-router";
import { Sidebar } from "./Sidebar";

export function Layout() {
  return (
    <div className="flex h-screen bg-bg text-text">
      <Sidebar />
      <div className="flex-1 min-w-0 h-full overflow-auto">
        <Outlet />
      </div>
    </div>
  );
}
