# Problem types

In the `wot` wire profile, and on the discovery routes in both profiles, errors are [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) problem details, served as `application/problem+json`. The framework's own problem types are documented here.

- **Each `type`** is this page's URL, `#`, and the type's name:

  ```text
  https://github.com/lucaswewa/teta-wot/blob/main/docs/problems.md#validation-error
  ```
- **Errors that are only HTTP's,** such as an unknown path or a method that isn't allowed, have the type `about:blank`, and the status's reason phrase as their title.
- **A problem type that a Thing chose** for its own error is kept, with its own title and status.
- **Every problem** has `type`, `title` and `status`, and a `detail` when there is one. Members that would be `null` are left out.

In the default `tetathing` profile, the same errors are answered as LabThings answers them: `{"detail": …}`, or FastAPI's `422` list for invalid input. The statuses below are the `wot` profile's.

## validation-error

**400 Bad Request.** The request's input isn't valid: a property's new value, an action's input, or a query argument to the Thing Description Directory.

- **`detail`** counts the issues, as in `"2 validation errors"`; for a query argument, it names it.
- **`invalid-params`** lists one entry for each issue:
  - `in`: `body`, `path` or `query`;
  - `name`: in the body, a JSON Pointer to the value, such as `/position/x`; otherwise the parameter's name;
  - `reason`: the message, worded as pydantic words it;
  - `code`: the kind of error, as pydantic names it (not given for query arguments).

## global-lock-busy

**503 Service Unavailable.** The global lock is held by another action, and the request waited for it briefly, then gave up ([Concurrency and hardware](guide/src/concurrency.md#the-global-lock)). `error` is `GlobalLockBusyError`.

## cancelled

**503 Service Unavailable.** The action, or the property's code, was cancelled before it finished. `error` is `InvocationCancelledError`.

## failed

**500 Internal Server Error.** The action, or the property's code, failed. `error` names the error, such as `InvocationError` or the name the Thing's code gave it, and `detail` gives its message.

## not-cancellable

**409 Conflict.** An invocation can't be cancelled: it has already completed, failed or been cancelled.

## no-output

**404 Not Found.** An invocation has no output to return: it hasn't completed, or its action returned nothing.

## Where they appear

- **As the answer** to a request that failed: a property read or write, a synchronous action, a cancellation, an invocation's output, a directory query.
- **In an invocation's `ActionStatus`,** as its `error`, when the action failed or was cancelled.
