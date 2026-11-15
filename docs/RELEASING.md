# Releasing

## Ship a version

1. Bump `version` in `Cargo.toml` and add a `CHANGELOG.md` entry.
2. Tag and push:

   ```
   git tag v1.0.0
   git push origin v1.0.0
   ```

[`release.yml`](../.github/workflows/release.yml) tests, builds, signs (once set up), hashes and attests
`lifehud.exe`, then publishes the GitHub release. Anyone can check where a download came from with
`gh attestation verify lifehud.exe -R George-API/LifeHUD`.

## Code signing (Azure Artifact Signing)

Unsigned, Windows SmartScreen warns on first run. Signing is the single biggest thing that makes the
download feel trustworthy. Signing steps in the workflow are skipped until the variables below exist.

1. In the [Azure portal](https://portal.azure.com), with a subscription, create an **Artifact Signing
   account** (formerly *Trusted Signing*). Note its region's endpoint, e.g.
   `https://eus.codesigning.azure.net/`. Check that individual developers are eligible in your country.
2. Complete **identity validation** for public trust. Microsoft verifies who you are; this can take a
   few days.
3. Create a **certificate profile** of type *Public Trust*.
4. Create an **app registration** in Microsoft Entra ID. Under *Certificates & secrets → Federated
   credentials*, add a GitHub Actions credential for repository `George-API/LifeHUD`, entity type
   **Environment**, name `release`. No client secret is needed.
5. On the signing account, assign that app the **Certificate Profile Signer** role.
6. In the GitHub repo, *Settings → Secrets and variables → Actions → Variables*, add:

   | Variable | Value |
   |---|---|
   | `AZURE_CLIENT_ID` | the app registration's client ID |
   | `AZURE_TENANT_ID` | your Entra tenant ID |
   | `AZURE_SUBSCRIPTION_ID` | the subscription holding the signing account |
   | `AZURE_SIGNING_ENDPOINT` | the endpoint from step 1 |
   | `AZURE_SIGNING_ACCOUNT` | the signing account name |
   | `AZURE_SIGNING_PROFILE` | the certificate profile name |

   Optionally, under *Settings → Environments → release*, require your approval before a release runs.

The next tag ships a signed exe, and the release notes drop the SmartScreen line on their own.
SmartScreen reputation builds with downloads, so the warning fades rather than vanishing on day one.

## winget

Life HUD is listed as `GeorgeAPI.LifeHUD`. Manifests live in [`packaging/winget`](../packaging/winget).
winget runs `lifehud.exe --install`, so it installs exactly like a normal download, and tracks it by
the `LifeHUD` entry in *Settings → Apps*.

**First version (once, by hand), after the GitHub release exists:**

1. Copy the SHA-256 from the release into `InstallerSha256` in
   `packaging/winget/GeorgeAPI.LifeHUD.installer.yaml`, and check the version and URL match the tag.
2. Validate, and optionally test locally:

   ```
   winget validate --manifest packaging\winget
   winget install --manifest packaging\winget
   ```

   (Local manifests need `winget settings --enable LocalManifestFiles` once, from an admin prompt.)
3. Submit with Microsoft's tool, using a GitHub classic token with the `public_repo` scope:

   ```
   winget install Microsoft.WingetCreate
   wingetcreate submit --token <token> packaging\winget
   ```

   This opens a pull request on [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).
   Automated checks run, then a moderator merges it, usually within a few days. After that,
   `winget install GeorgeAPI.LifeHUD` works for everyone.

**Every version after that is automatic:** fork `microsoft/winget-pkgs` to your account, create a
classic token with `public_repo` and `workflow` scopes, and save it as the repository secret
`WINGET_TOKEN`. Each tag then opens the winget update pull request by itself.
