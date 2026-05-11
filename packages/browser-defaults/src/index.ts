/**
 * Browser Default Constraint Modules
 *
 * Provides the default CSS/HTML layout constraints for each browser engine
 * as ViewScript IR modules.
 */

export type BrowserEngine = 'chromium' | 'firefox' | 'webkit';

export interface BrowserDefaultModule {
  engine: BrowserEngine;
  version: string;
  constraints: unknown[]; // VS IR constraints
}

export async function loadBrowserDefaults(engine: BrowserEngine): Promise<BrowserDefaultModule> {
  // TODO: Load browser-specific default constraints
  return {
    engine,
    version: '1.0.0',
    constraints: [],
  };
}
