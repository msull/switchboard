Fixed: A panic on a background thread no longer takes down the window through a poisoned lock, and a content read that panics reports failed instead of stopping the worker.
