Changed: A commit made in a Dispatch lane now runs the project's .githooks (set once on the clone, existing clones included), while Dispatch's own git calls still run no hook.
