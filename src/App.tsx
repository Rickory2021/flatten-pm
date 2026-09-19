// src/App.tsx
import { MemoryRouter, Routes, Route, Navigate } from "react-router";
import { Layout } from "@/components/layout/Layout";
import { ReposPage } from "@/pages/ReposPage";
import { RecipesPage } from "@/pages/RecipesPage";
import { TransformsPage } from "@/pages/TransformsPage";
import { TemplatesPage } from "@/pages/TemplatesPage";
import { BindingsPage } from "@/pages/BindingsPage";
import { WatchPage } from "@/pages/WatchPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { DevToolsPage } from "@/pages/DevToolsPage";

export default function App() {
  return (
    <MemoryRouter initialEntries={["/repos"]}>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<Navigate to="/repos" replace />} />
          <Route path="repos" element={<ReposPage />} />
          <Route path="recipes" element={<RecipesPage />} />
          <Route path="transforms" element={<TransformsPage />} />
          <Route path="templates" element={<TemplatesPage />} />
          <Route path="bindings" element={<BindingsPage />} />
          <Route path="watch" element={<WatchPage />} />
          <Route path="settings" element={<SettingsPage />} />
          <Route path="dev-tools" element={<DevToolsPage />} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}
