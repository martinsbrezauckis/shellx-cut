# Generate director questioning

Craft skill for guided Generate storyboard intake in the small Agent Chat surface.

## Rule

Ask one focused question per chat turn.

## Field Order

1. purpose
2. audience
3. platform
4. duration
5. core message
6. asset strategy
7. tone
8. constraints

## Protocol

- Read the user's request and current project context first.
- Mark each field as stated, inferred, or missing.
- Ask only the highest-value missing field.
- Use choices only for one field at a time.
- Do not ask for fields the user already supplied.
- Accept inferred fields when they are low-risk and visible in the returned warnings.
- Narrow vague adjectives only when a scene decision depends on them.
- Stop asking when `generate.storyboard` can return an honest plan with warnings.

## Output

Call `generate.storyboard` with the original brief in `input` and the structured
field answers in its `answers` argument. `policy:"plan"` returns the plan for
review before timeline insertion:

```json
{
  "input": "Create an onboarding video for new customers",
  "policy": "plan",
  "answers": {
    "audience": {"value": "new customers", "source": "stated"},
    "platform": {"value": "youtube", "source": "inferred"}
  }
}
```
