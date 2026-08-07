// Narrowly aimed at one class of bug: a Rules-of-Hooks violation is invisible
// to `tsc` and to every test, and at runtime React aborts the whole tree —
// which presents as a blank window with no error at all. A conditional early
// return sitting between hooks shipped and went unnoticed exactly that way.
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  { ignores: ["dist/**", "src-tauri/**", "public/**", "preview/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // The one that matters: breaking it takes the whole UI down silently.
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      // Loading data on mount is exactly this app's shape; the rule's advice
      // (fetch in an event handler or a framework loader) doesn't apply here.
      "react-hooks/set-state-in-effect": "off",
    },
  },
);
