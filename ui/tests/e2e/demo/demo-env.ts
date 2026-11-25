// Must be the first import of every demo file: env.ts reads MG_QA_ROOT at import time.
// App state lives under OUT\work (never in the repo): env.ts then puts its WORK at OUT\work\work.
export const OUT = process.env.MG_DEMO_OUT ?? 'D:\\own\\saas\\qa-live\\demo'
process.env.MG_QA_ROOT = `${OUT}\\work`
process.env.MG_QA_DAY ??= 'demo'
export * from '../qa-live/env'
