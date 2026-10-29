# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub:
**[Report a vulnerability](https://github.com/DorianDevp/tuitor/security/advisories/new)**
(Security tab → Advisories). Do not open a public issue.

Include what is affected, how to reproduce it, and the impact you see. You will get an answer within
a week.

## Supported versions

Fixes go into the latest release.

## Scope

tuitor starts debug adapters and the programs they debug, downloads adapters with `tuitor install`
(after asking), and runs language servers. Issues in how tuitor does these are in scope. Issues in the
adapters themselves (Xdebug, js-debug, Delve, gdb, LLDB) belong to their projects.
