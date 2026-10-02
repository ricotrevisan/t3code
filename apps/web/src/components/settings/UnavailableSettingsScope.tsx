import { useNavigate } from "@tanstack/react-router";

import { Button } from "../ui/button";
import { Alert, AlertDescription } from "../ui/alert";
import { useNavigateToMainApp } from "../sidebar/mainAppLocation";
import { SettingsPageContainer } from "./settingsLayout";
import { useSettingsScope } from "./SettingsScopeContext";
import { useSettingsProjectGroups } from "./useSettingsProjectGroups";

/** Keep stale settings links read-only until the user explicitly chooses a new target. */
export function UnavailableSettingsScope({ message }: { message: string }) {
  const { search } = useSettingsScope();
  const groups = useSettingsProjectGroups();
  const navigate = useNavigate({ from: "/settings" });
  const navigateToMainApp = useNavigateToMainApp();
  const project = groups.find((group) => group.projectKey === search.project);
  const openProjects = (projectKey?: string) => {
    void navigate({
      to: "/settings/projects",
      // Explicit axes prevent the scope-retention middleware from restoring stale filters.
      search: () => ({ project: projectKey, machine: undefined, checkout: undefined }),
      hash: "",
    });
  };

  return (
    <SettingsPageContainer>
      <Alert role="status">
        <AlertDescription>
          <p>{message}</p>
          <div className="flex flex-wrap gap-2">
            {project && project.memberProjects.length > 0 ? (
              <Button size="sm" variant="outline" onClick={() => openProjects(project.projectKey)}>
                Open project settings
              </Button>
            ) : null}
            <Button size="sm" variant="outline" onClick={() => openProjects()}>
              Choose another project
            </Button>
            <Button size="sm" variant="outline" onClick={() => void navigateToMainApp()}>
              Back to threads
            </Button>
          </div>
        </AlertDescription>
      </Alert>
    </SettingsPageContainer>
  );
}
