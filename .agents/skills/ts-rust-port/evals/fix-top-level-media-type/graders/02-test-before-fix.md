---
type: tool_order
before: { tool: Bash, input_match: 'cargo\S*\s+test' }
after: { tool: Edit, input_match: 'src/lib\.rs' }
---
