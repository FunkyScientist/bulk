# Security policy

Bulk runs a helper with administrator rights and can delete files, so security
reports are taken seriously.

## Reporting a vulnerability

Please **don't open a public issue**. Report it privately instead:

1. Go to the [Security tab](https://github.com/FunkyScientist/bulk/security) of this repository.
2. Click **Report a vulnerability**.

Include what you found, the steps to reproduce it, and the Bulk version. You should
get a reply within a week. Once a fix is released, you'll be credited in the
release notes unless you'd rather not be.

## What counts

Things that are especially worth reporting:

- a way for another program or user to get admin rights through the helper,
  its scheduled task or its named pipe,
- the helper writing, changing or deleting anything other than its own task and logs,
- deletes reaching files they shouldn't, such as getting past the CLI's protected
  system, program and profile folders,
- the installer leaving files that run elevated somewhere normal programs can write to.

The [Security model](README.md#security-model) section of the README describes how
Bulk is meant to behave.

## Supported versions

Only the latest release gets security fixes.
