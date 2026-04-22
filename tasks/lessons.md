# Lessons

- When rendering Jira Atlassian Document Format for CLI output, preserve structured metadata such as link marks and smart-link URLs; flattening only `text` nodes drops important ticket content.
- For user-facing CLI features, add end-to-end tests that invoke the `jit` binary against a local fake Jira server; helper-only unit tests can miss command dispatch, config-loading order, and rendered output regressions.
