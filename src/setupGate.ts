/** Finish setup is available only after both required first-run steps. */
export function setupCanFinish(
  hasApiKey: boolean,
  nexusWebsiteSignedIn: boolean,
): boolean {
  return hasApiKey && nexusWebsiteSignedIn;
}
