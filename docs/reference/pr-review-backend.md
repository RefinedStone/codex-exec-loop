# Configure defensive pull request review

The existing GitHub watcher reviews its selected repository and pull request. Review is off by default. To enable it, configure a model route, provision a dedicated OpenAI API key for that repository, and confirm the repository mode. The API account must have access to a text model on the Responses API. Desktop and Codex login sessions do not supply this credential.

1. Add a backend and route to `$AKRA_HOME/delegation.toml`. Use a model ID available to your API account. For example:

   ```toml
   [backend.defensive]
   kind = "openai_text"
   model = "gpt-5-codex"
   enabled = true
   tool_access = "none"
   max_concurrent = 1

   [feature.pr_review]
   mode = "observe"
   route = ["defensive"]
   deadline = "2m"
   ```

   A repository can narrow the user policy in `.akra/delegation.toml`. The shared delegation budget, deadline, backend concurrency limit, and circuit apply.

2. From the repository checkout, pipe the API key from your secret manager to `akra pr-review-settings set-api-key`. Akra accepts it on standard input and saves it under `akra-pr-review-openai` in the OS credential store. The account name is the lowercase GitHub `owner/repo`. The watcher must run as the same OS user with access to that store. Linux requires an available Secret Service.

3. Run `akra pr-review-settings observe` in a terminal. Type the displayed repository confirmation. `observe` stores validated findings locally. To allow an aggregate GitHub comment, set the feature policy to `on` and confirm `akra pr-review-settings on`. Run `akra pr-review-settings off` to disable review. The existing GitHub watcher needs read access to the selected PR and write access for comments.

Review sends the bounded PR title, body, and diff to `api.openai.com`. Akra masks recognized credential patterns before sending. Review the repository's disclosure policy before enabling this external transfer. The request sets `store` to `false`, declares an empty tool list, and disables tool choice. The watcher accepts only a completed text result that passes its findings schema. It retains the revision identity, route metadata, and validated result locally. A lost response cannot trigger an automatic repeat of the same attempt.
