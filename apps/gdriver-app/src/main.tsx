import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "./i18n";
import App from "./App";
import "./styles.css";
import { useAppStore } from "./store/appStore";

// Expose store in dev console for debugging (e.g. __appStore.resetOnboarding())
if (import.meta.env.DEV) {
  (window as unknown as Record<string, unknown>).__appStore = useAppStore;
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
