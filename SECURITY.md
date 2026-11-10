# Security

Scrawl is local-first. The only network feature is the optional **tablet server**, which lets a
phone or tablet act as a drawing tablet. It is **off by default**; when off there are no threads and
no sockets. This document describes the threat model, the design, and how to reset everything.

## Threat model

Protected assets: your notes (they never leave your machine) and your computer's integrity.

In scope:
- Another device on your Wi-Fi trying to use the tablet server (eavesdropping, injecting ink, guessing
  pairing tokens, replaying old sessions, cross-site WebSocket attacks from a web page you visit).
- A leaked private-CA key being used to impersonate real websites.
- Malformed or hostile messages from a paired device.

Out of scope: a compromised PC or compromised paired tablet (a paired device can draw into the active
block, by design), physical access to an unlocked machine, and the security of your Wi-Fi itself.

## Design

1. **TLS everywhere, 1.3 only.** `rustls` (no OpenSSL); the build does not even compile TLS 1.2 support.
   First enable generates a local root CA (`rcgen`). The CA has X.509 **Name Constraints** permitting only
   `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10` (Tailscale) and `.local`, so a leaked key
   cannot be used to impersonate a real website (tested: a CA-signed certificate for `example.com` fails
   verification). The CA private key is stored in the OS keyring (Secret Service on Linux, Credential
   Manager on Windows), never as a plaintext file; only the public certificate is written to disk. Server
   certificates are valid for 7 days and are re-issued in their last 2 days while the server runs (new
   connections get the new certificate without a restart); a restart re-issues when the LAN IPs change.
2. **One-time CA setup.** A separate plain-HTTP bootstrap endpoint serves only the CA certificate
   (`.mobileconfig` for iOS, `.crt` for Android). It exists only while the setup screen is open and closes
   after one download or 5 minutes. The PC shows the CA SHA-256 fingerprint so you can compare it on the
   device.
3. **Pairing.** The QR code contains the HTTPS URL plus a single-use 128-bit token in the URL *fragment*
   (browsers never send fragments, so it is not in any log or request). The token expires after 2 minutes.
   After scanning, both screens show the same 6-digit code and you must press Approve on the PC. Approved
   devices receive a random 256-bit secret (only its SHA-256 is stored on the PC); it is rotated at every
   connection, devices unused for 30 days expire, and each device can be revoked. Revoking cuts off a
   device that is connected at that moment, not only future connections (tested end to end). Five failed pairing
   attempts lock pairing for 5 minutes. A wrong token guess burns the token.
4. **Least privilege.** The protocol has only: pair/authenticate, strokes, text, and UI commands for the
   active block. There is no message to read, list, export or delete notes and no filesystem access.
   Every message is size-limited (32 KB), schema-checked (`deny_unknown_fields`), range-checked
   (<= 256 points per batch, coordinate/pressure/tilt bounds, text length) and rate-limited; malformed input
   is dropped without crashing (fuzz-tested). WebSocket upgrades require `Origin: https://<our host>`.
   Responses carry a strict Content-Security-Policy, HSTS, `nosniff`, `no-referrer`, `no-store`.
   The web client is vanilla JS embedded in the binary and makes no third-party requests. The only
   thing sent back to the device is the active expression (its candidates and a PNG render, `Frame`).
   Pen tests in `crates/remote/tests/e2e.rs` cover path traversal on static routes, oversized frames,
   huge point counts and floods from unpaired sockets.
5. **Network exposure.** The server binds only to private-range interfaces (LAN and Tailscale), never to
   public interfaces. No UPnP, no port forwarding, no cloud relay. Away from home, use Tailscale: the
   tablet reaches the PC at its `100.x.y.z` address, which is inside the CA's permitted range. The app shows
   a "tablet server running" indicator with a Stop button whenever the server is on, and the server
   stops itself after 15 minutes without activity (configurable) and when the app closes.

## Setting up a device (one time)

Open `http://<PC address>:8080/` on the device while the setup screen is open.

- **iPhone / iPad:** tap "iPhone / iPad profile", allow the download, then install it from the "Profile
  Downloaded" entry at the top of Settings (or Settings > General > VPN & Device Management). Then enable
  trust under Settings > General > About > Certificate Trust Settings
  ([Apple](https://support.apple.com/en-in/102390)). One forum report says a self-made CA was rejected on
  iOS 26.5.2 ([Start9 forum](https://community.start9.com/t/enable-trusted-certificate-on-iphone-ios-version-26-5-2/5411));
  this is unconfirmed and **has not been tested by the authors on a real device**.
- **Android:** tap "Android certificate (.crt)", then Settings > Security & privacy > More security &
  privacy > Encryption & credentials > Install a certificate > CA certificate > Install anyway, and pick
  the downloaded file. Menu names vary by manufacturer. Apps that ignore user-installed CAs are
  unaffected here because the tablet page runs in the browser. **Not tested on a real device.**

Compare the fingerprint shown on the PC with the device's certificate details.

## Resetting

- **All pairing:** in the app, Tablet > Paired devices > Revoke all (or delete `devices.json` in the
  config folder, `~/.config/scrawl/` on Linux, `%APPDATA%\scrawl\` on Windows).
- **The CA:** stop the server, delete `ca.pem` from that folder, and remove the keyring entry
  "scrawl-tablet-server / root-ca-key" (Seahorse/`secret-tool` on Linux, Credential Manager on Windows).
  A new CA is created next time; remove the old profile/certificate from your devices.

## Reporting

Open a private security advisory on the GitHub repository.
