import { MetaProvider } from "@solidjs/meta";
import { Router } from "@solidjs/router";
import { FileRoutes } from "@solidjs/start/router";
import { Show, Suspense } from "solid-js";
import Shell from "./components/Shell";
import { OwnerProvider } from "./lib/owner";
import { WorkgroupsProvider } from "./lib/workgroups";
import "./app.css";

export default function App() {
  return (
    <Router
      root={(props) => (
        <MetaProvider>
          <Suspense>
            <OwnerProvider>
              <WorkgroupsProvider>
                {/* The glasses' page draws on the glasses; the phone holding it shows no console. */}
                <Show when={!(props.location.pathname === "/g2" || props.location.pathname.startsWith("/g2/"))} fallback={props.children}>
                  <Shell>{props.children}</Shell>
                </Show>
              </WorkgroupsProvider>
            </OwnerProvider>
          </Suspense>
        </MetaProvider>
      )}
    >
      <FileRoutes />
    </Router>
  );
}
