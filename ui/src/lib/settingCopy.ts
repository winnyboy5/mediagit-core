// Plain-words labels and descriptions for the settings people actually meet. A key not listed
// here falls back to the CLI's own wording. Environment variable names go on the muted key line.
export interface SettingCopy {
  label: string
  description: string
  env?: string
}

export const SETTING_COPY: Record<string, SettingCopy> = {
  'author.name': { label: 'Your name', description: 'Shown on every version you save.' },
  'author.email': { label: 'Your email', description: 'Shown on every version you save.' },
  'performance.upload_concurrency': { label: 'Files sent at once', description: 'How many pieces are sent in parallel when you push. More is faster on a fast connection.', env: 'MEDIAGIT_UPLOAD_CONCURRENCY' },
  'performance.download_concurrency': { label: 'Files received at once', description: 'How many pieces are fetched in parallel when you pull or fetch.', env: 'MEDIAGIT_DOWNLOAD_CONCURRENCY' },
  'storage.create_dirs': { label: 'Create missing folders', description: 'For storage on this computer: make folders that do not exist yet.' },
  'storage.sync': { label: 'Wait for the disk after each write', description: 'For storage on this computer: slower, but safer if the power goes out.' },
  'storage.file_permissions': { label: 'Who can open stored files', description: 'For storage on this computer, as a permission code such as 644.' },
  'storage.region': { label: 'Bucket region', description: 'For S3 storage: the region your bucket lives in.' },
  'storage.credentials_path': { label: 'Google key file', description: 'For Google Cloud storage: the service-account file. Leave empty to use your signed-in Google account.' },
  'storage.secret_access_key': { label: 'S3 secret key', description: 'For S3 storage: the secret half of your access key.' },
  'remotes.<name>.url': { label: 'Server address', description: 'Where this remote lives, starting with http:// or https://.' },
  'remotes.<name>.token': { label: 'Access token', description: 'Lets you sign in to this remote. It is saved as plain text in the repository settings.' },
}
