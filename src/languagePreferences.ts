export const LANGUAGE_SETUP_VERSION = 1;

export function hasSavedLanguage(settings: {
  hasChosenLanguage?: unknown;
  languageSetupVersion?: unknown;
  metadataLanguage?: unknown;
}) {
  return settings.languageSetupVersion === LANGUAGE_SETUP_VERSION &&
    settings.hasChosenLanguage === true &&
    (settings.metadataLanguage === "arabic" || settings.metadataLanguage === "english");
}
