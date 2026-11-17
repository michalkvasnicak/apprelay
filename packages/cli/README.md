# apprelay

Requires Node.js 24.12 or newer.

## Upload

```sh
npx apprelay ./MyApp.ipa
pnpm dlx apprelay ./MyApp.ipa
npm install --global apprelay
apprelay ./MyApp.ipa
apprelay --help
```

## Server configuration

Default: `https://apprelay.brosoft.sk`. Remote servers require HTTPS; localhost permits HTTP.

```sh
apprelay --server https://apprelay.example.com ./MyApp.ipa
APPRELAY_SERVER=http://localhost:8080 apprelay ./MyApp.ipa
apprelay ./MyApp.ipa > share-url.txt
```

## Local development

```sh
pnpm install
pnpm cli ./MyApp.ipa
node packages/cli/bin/apprelay.mjs ./MyApp.ipa
pnpm --filter apprelay api:generate
pnpm --filter apprelay api:check
```

## Publish

The npm package name `apprelay` requires registry availability and publishing access.

```sh
pnpm --dir packages/cli publish --access public
```
