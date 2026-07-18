## Summary

Describe the user-visible change and why it belongs in MCP Kali.

## Verification

- [ ] `make verify`
- [ ] Relevant security checks and tests were added or updated
- [ ] Documentation and `CHANGELOG.md` were updated when behavior changed

## Security and compatibility

- [ ] No credentials, real targets, customer data, scanner evidence, or `.env` files are included
- [ ] MCP stdout remains machine-readable and diagnostics remain on stderr
- [ ] External output remains untrusted data and is escaped where rendered
- [ ] Commands continue to use structured process arguments without a shell
- [ ] Public JSON/API compatibility was preserved or the migration is documented
