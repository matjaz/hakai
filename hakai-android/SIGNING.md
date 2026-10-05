# Signing the Android release

Android only installs an update over an existing app when both are signed with the **same
key**. Without a release key, CI signs every APK with a throwaway debug key made on the
runner — so each release would need the previous one uninstalled first. One release key,
made once and kept forever, fixes that.

**Lose the keystore or its password and no future release can update existing installs** —
users would have to uninstall and reinstall. Back up both (step 2) before anything else.

## 1. Create the keystore (once)

Somewhere outside the repository — never commit it:

```sh
keytool -genkeypair -v \
  -keystore ~/hakai-release.keystore -storetype PKCS12 \
  -alias hakai -keyalg RSA -keysize 4096 -validity 10000 \
  -dname "CN=Hakai"
```

`keytool` (from the JDK — `/opt/homebrew/opt/openjdk@17/bin` on this Mac) asks for a
password. With PKCS12 the key's password is the keystore's, so there's only one.
`-validity 10000` is ~27 years; Google Play requires validity past 2033.

Check it:

```sh
keytool -list -v -keystore ~/hakai-release.keystore | grep -E "Alias|SHA256"
```

Note the `SHA256` fingerprint — it's how you recognise this key later.

## 2. Back it up

- the file `~/hakai-release.keystore` → somewhere safe and off this machine (a password
  manager's file attachment, an encrypted backup);
- the password → the password manager.

## 3. Give it to CI

The `hakai-android` job reads two repository secrets. With the GitHub CLI:

```sh
base64 -i ~/hakai-release.keystore | gh secret set ANDROID_KEYSTORE_BASE64 -R matjaz/hakai
gh secret set ANDROID_KEYSTORE_PASSWORD -R matjaz/hakai     # prompts for the password
```

(Or in the browser: repository → *Settings* → *Secrets and variables* → *Actions* → *New
repository secret*, pasting the output of `base64 -i ~/hakai-release.keystore`.)

From the next run on, CI decodes the keystore into the runner's temp directory and
`package.sh` signs with it. Secrets aren't available to pull requests from forks; those
builds keep using a debug key, which is fine — they aren't released.

## 4. Build signed locally (optional)

```sh
ANDROID_KEYSTORE=~/hakai-release.keystore \
ANDROID_KEYSTORE_PASSWORD='…' \
hakai-android/package.sh
```

Without `ANDROID_KEYSTORE`, `package.sh` signs with the local debug key
(`~/.android/debug.keystore`) and says so.

## 5. Verify a release APK

```sh
"$ANDROID_HOME"/build-tools/*/apksigner verify --print-certs hakai-<version>-android-arm64.apk
```

The `SHA-256 digest` must match the fingerprint from step 1. A `CN=Android Debug` signer
means the secrets weren't picked up.

## Switching over

APKs installed so far (local builds, or releases before the secrets existed) are signed
with a debug key. Uninstall that once before installing the first release-signed APK;
every release after that updates in place.

If hakai ever goes to Google Play, this key becomes the *upload key*: Play App Signing then
holds the actual app-signing key and accepts uploads signed with this one.
