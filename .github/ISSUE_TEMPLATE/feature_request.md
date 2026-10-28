name: Feature request
description: Suggest a capability for SpurCode
labels: ["enhancement"]
body:
  - type: textarea
    id: problem
    attributes:
      label: What problem are you trying to solve?
      description: The orchestration workflow or friction point this feature would address.
      placeholder: When I ..., I currently have to ...
    validations:
      required: true
  - type: textarea
    id: solution
    attributes:
      label: What would the ideal solution look like?
      description: A description of the behavior, surface (TUI / CLI / MCP), or config you'd expect.
    validations:
      required: true
  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives you have considered
      description: Workarounds, other tools, or configurations that almost cover the need.
    validations:
      required: false
  - type: dropdown
    id: area
    attributes:
      label: Which area does this touch?
      multiple: true
      options:
        - Delegation / plans
        - Worker agents (ACP)
        - TUI
        - Code graph / context
        - Cost / analytics
        - Project management integration
        - Installation / packaging
        - Other
    validations:
      required: false
