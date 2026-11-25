// Must be the first import of every demo file: env.ts reads MG_QA_ROOT at import time.
// Clips and narration live under OUT. App state lives under D:\Studio\MediaGit (shown on screen, never in the repo):
// env.ts then puts its WORK at D:\Studio\MediaGit\work.
export const OUT = process.env.MG_DEMO_OUT ?? 'D:\\own\\saas\\qa-live\\demo'
export const DEMO_ROOT = 'D:\\Studio\\MediaGit'
process.env.MG_QA_ROOT = DEMO_ROOT
process.env.MG_QA_DAY ??= 'demo'
export * from '../qa-live/env'
import { CLI_ENV } from '../qa-live/env'
// The user's own work in the demo is Alex Rivera (CLI_ENV is spread into the app and seed CLI env at use time).
CLI_ENV.MEDIAGIT_AUTHOR_NAME = 'Alex Rivera'
CLI_ENV.MEDIAGIT_AUTHOR_EMAIL = 'alex@studio.test'
