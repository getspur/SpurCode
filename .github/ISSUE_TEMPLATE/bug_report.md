name: Bug report
description: Something in SpurCode is broken or behaving unexpectedly
labels: ["bug"]
body:
  - type: markdown
    attributes:
      value: |
        Thanks for taking the time to report a bug. Before opening an issue,
        please search existing issues to avoid duplicates.
  - type: textarea
    id: what-happened
    attributes:
      label: What happened?
      description: A clear description of the bug and what you expected to happen instead.
      placeholder: Tell us what you see and what you expected.
    validations:
      required: true
  - type: textarea
    id: reproduce
    attributes:
      label: Steps to reproduce
      description: Minimal command sequence or interaction in the TUI.
      placeholder: |
        1. Run `spur ...`
        2. ...
    validations:
      required: true
  - type: input
    id: version
    attributes:
      label: SpurCode version
      description: Output of `spur --version`
      placeholder: e.g. spur-cli 1.0.4
    validations:
      required: true
  - type: textarea
    id: environment
    attributes:
      label: Environment
      description: OS, shell, agent CLIs involved (with versions), PM integration if any.
      placeholder: macOS 15, zsh, claude-code 2.x / codex 0.x, beads
    validations:
      required: true
  - type: textarea
    id: logs
    attributes:
      label: Relevant logs or session evidence
      description: |
        Terminal output, TUI transcript, or `.spur/` artifacts. Redact API keys
        and secrets before pasting.
        This will be formatted as code, so no backticks needed.
      render: shell
    validations:
      required: false
