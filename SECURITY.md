# Security

## Threat model

ChatLocal is designed for a network you trust, such as a home, office or
classroom LAN. Traffic between computers is **not encrypted** and peers are
**not authenticated**: anyone on the same network can read messages, receive
files, or connect with any display name. This is a deliberate trade-off for a
zero-configuration tool, not a bug. Do not use ChatLocal on a public or hostile
network.

Received files are written to the application data folder exactly as sent.
Treat them like any other file from another computer.

## Reporting a vulnerability

If you find a problem that goes beyond the limits above, for example a crash or
memory issue triggered by a malformed frame, a path traversal in file handling,
or a way to read files outside the data folder, please report it privately
through [GitHub Security Advisories](https://github.com/iniadil/chat-local/security/advisories/new)
rather than a public issue. You should get a reply within a week.
