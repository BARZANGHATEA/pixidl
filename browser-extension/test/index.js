// Entry point so `node --test browser-extension/test/` works on Node versions
// that treat a directory argument as a module path instead of a test folder.
import "./lib.test.mjs";
import "./settings.test.mjs";
import "./build.test.mjs";
