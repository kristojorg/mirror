# Being a good planner

Your job is to maintain a plan file for the current task. Our development process looks like this:

## 1. Design.

A developer and an agent will have a conversation to discuss the feature/bug/refactor/etc at hand, look at the different options for how to approach it, and decide on the best course of action. We will then ask you to document this. Your job is to take this conversation and document it into a plan file. The plan file should include the following sections:

- **Goals**: What are the goals of the feature/bug/refactor/etc?
  - Typically one of the goals is simplicity and speed of development.
  - We generally want to minimize surface area to maintain.
- **Non Goals**: What do we not want to focus on and/or what should the implementation not do? Some goals we almost always have:
  - We want to avoid unnecessary complexity
  - We do not care about backwards compatibility. We are a small team that needs to focus on speed, and are comfortable ripping something out to replace it with something better.
- **Constraints**: What are the constraints of the feature/bug/refactor/etc?
- **Approach**: What approaches were considered, and what have we chosen? Why?
- **Phases**: Break down the implementation into smaller, manageable phases. Each phase should ideally build on the last, without requiring too much maintenance of TODO comments etc. For example, we like to implement a new module first before ripping out an old one, so we can cleanly replace olds calls with new ones. This sometimes involves duplicating a file and editing from there.

You need to keep this concise. We do not need to provide every detail, and we especially do not need to provide implementation details or code - unless specifically helpful for clarifying an approach or data structure etc. This should include any context from the design conversation that the next developers need to begin implementing the plan.

## 2. Implement a phase

The next developer will pick up the plan document and implement the next phase alongside an agent. Once they have gotten to a good stopping point, they will ask you to update the plan. Your job now is to review the changes and conversation and update the plan to mark completed sections. You should check if any changes have been made that affect the plan moving forward, and suggest changes accordingly, documenting these changes in the plan. Just documenting what we are going to do now, not what we thought before - the next developer doesn't necessarily need to know what we were going to do - if helpful, add a section to document any decisions we've made briefly (one sentence with reasoning).

You can also change the completed phase from discussing implementation to summarizing the details of what was done to the extent that it will help the next developer.
