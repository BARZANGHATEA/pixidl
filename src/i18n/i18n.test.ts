import en from "./en.json";
import fa from "./fa.json";
import { dirFor } from "./index";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

function keys(o: Record<string, unknown>, p = ""): string[] {
  return Object.entries(o).flatMap(([k, v]) => (v && typeof v === "object" ? keys(v as Record<string, unknown>, `${p}${k}.`) : [`${p}${k}`]));
}
function placeholders(s: string) {
  return [...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();
}

describe("translations", () => {
  it("English and Persian have identical keys", () => {
    expect(keys(fa).sort()).toEqual(keys(en).sort());
  });

  it("placeholders match", () => {
    const get = (o: Record<string, unknown>, k: string) => k.split(".").reduce<unknown>((a, p) => (a as Record<string, unknown>)[p], o) as string;
    for (const k of keys(en)) expect(placeholders(get(fa, k)), k).toEqual(placeholders(get(en, k)));
  });

  it("every t('…') key used in the source exists", () => {
    const all = new Set(keys(en).map((k) => k.replace(/_(one|other)$/, "")));
    const walk = (dir: string): string[] =>
      readdirSync(dir).flatMap((f) => {
        const p = join(dir, f);
        return statSync(p).isDirectory() ? walk(p) : /\.tsx?$/.test(f) && !f.includes(".test.") ? [p] : [];
      });
    const missing: string[] = [];
    for (const file of walk(join(__dirname, ".."))) {
      const src = readFileSync(file, "utf8");
      for (const m of src.matchAll(/\bt\(\s*"([a-zA-Z0-9_.]+)"/g)) if (!all.has(m[1])) missing.push(`${file}: ${m[1]}`);
    }
    expect(missing).toEqual([]);
  });

  it("Persian is right-to-left", () => {
    expect(dirFor("fa")).toBe("rtl");
    expect(dirFor("en")).toBe("ltr");
  });
});
