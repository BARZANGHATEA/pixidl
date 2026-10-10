import en from "./en.json";
import { AVAILABLE_LANGUAGES, LANGUAGES, dirFor, intlLocale } from "./index";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

type Dict = Record<string, unknown>;
const PLURAL = /_(zero|one|two|few|many|other)$/;

function keys(o: Dict, p = ""): string[] {
  return Object.entries(o).flatMap(([k, v]) => (v && typeof v === "object" ? keys(v as Dict, `${p}${k}.`) : [`${p}${k}`]));
}
function placeholders(s: string) {
  return [...new Set([...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]))].sort();
}
const get = (o: Dict, k: string) => k.split(".").reduce<unknown>((a, p) => (a as Dict | undefined)?.[p], o) as string | undefined;

const locales: Record<string, Dict> = Object.fromEntries(
  readdirSync(__dirname)
    .filter((f) => f.endsWith(".json"))
    .map((f) => [f.replace(/\.json$/, ""), JSON.parse(readFileSync(join(__dirname, f), "utf8")) as Dict]),
);
const enBase = new Set(keys(en).map((k) => k.replace(PLURAL, "")));
const pluralBases = [...new Set(keys(en).filter((k) => PLURAL.test(k)).map((k) => k.replace(PLURAL, "")))];

describe("translations", () => {
  it("every locale file is a listed language", () => {
    for (const code of Object.keys(locales)) expect(LANGUAGES.map((l) => l.code), code).toContain(code);
  });

  for (const [code, dict] of Object.entries(locales)) {
    if (code === "en") continue;
    describe(code, () => {
      it("has exactly the English keys", () => {
        const base = new Set(keys(dict).map((k) => k.replace(PLURAL, "")));
        expect([...enBase].filter((k) => !base.has(k)), "missing").toEqual([]);
        expect([...base].filter((k) => !enBase.has(k)), "extra").toEqual([]);
      });

      it("has every plural form the language needs", () => {
        const cats = new Intl.PluralRules(intlLocale(code)).resolvedOptions().pluralCategories;
        for (const b of pluralBases) for (const c of cats) expect(get(dict, `${b}_${c}`), `${b}_${c}`).toBeTypeOf("string");
      });

      it("keeps placeholders and has no empty strings", () => {
        for (const k of keys(dict)) {
          const value = get(dict, k)!;
          expect(value.trim().length, k).toBeGreaterThan(0);
          const source = get(en, k) ?? get(en, k.replace(PLURAL, "_other"));
          if (source !== undefined) expect(placeholders(value), k).toEqual(placeholders(source));
        }
      });
    });
  }

  it("every t('…') key used in the source exists", () => {
    const walk = (dir: string): string[] =>
      readdirSync(dir).flatMap((f) => {
        const p = join(dir, f);
        return statSync(p).isDirectory() ? walk(p) : /\.tsx?$/.test(f) && !f.includes(".test.") ? [p] : [];
      });
    const missing: string[] = [];
    for (const file of walk(join(__dirname, ".."))) {
      const src = readFileSync(file, "utf8");
      for (const m of src.matchAll(/\bt\(\s*"([a-zA-Z0-9_.-]+)"/g)) if (!enBase.has(m[1])) missing.push(`${file}: ${m[1]}`);
    }
    expect(missing).toEqual([]);
  });

  it("direction follows the script", () => {
    for (const code of ["fa", "ar", "ckb"]) expect(dirFor(code)).toBe("rtl");
    for (const code of ["en", "kmr", "tr", "zh"]) expect(dirFor(code)).toBe("ltr");
  });

  it("only languages with a file are offered", () => {
    expect(AVAILABLE_LANGUAGES.map((l) => l.code).sort()).toEqual(Object.keys(locales).sort());
  });
});
