import assert from "node:assert/strict";
import test from "node:test";

import { renderNotices, shippedPackageIds } from "./third-party-notices.mjs";

const normal = [{ kind: null, target: null }];
const dev = [{ kind: "dev", target: null }];
const build = [{ kind: "build", target: null }];

test("only normal dependencies reachable from the root ship", () => {
  const metadata = {
    resolve: {
      root: "root",
      nodes: [
        { id: "root", deps: [{ pkg: "a", dep_kinds: normal }, { pkg: "t", dep_kinds: dev }, { pkg: "b", dep_kinds: build }] },
        { id: "a", deps: [{ pkg: "c", dep_kinds: normal }] },
        { id: "t", deps: [{ pkg: "d", dep_kinds: normal }] },
        { id: "b", deps: [] },
        { id: "c", deps: [] },
        { id: "d", deps: [] },
      ],
    },
  };
  assert.deepEqual([...shippedPackageIds(metadata)].sort(), ["a", "c"]);
});

test("identical license texts are written once with every user listed", () => {
  const packages = [
    { name: "zeta", version: "1.0.0", license: "MIT" },
    { name: "alpha", version: "2.0.0", license: "MIT" },
  ];
  const texts = { zeta: ["MIT text\n"], alpha: ["MIT text\n"] };
  const output = renderNotices(packages, (pkg) => texts[pkg.name]);

  assert.equal(output.match(/MIT text/g).length, 1);
  assert.match(output, /Used by:\n {2}alpha 2\.0\.0 \(MIT\)\n {2}zeta 1\.0\.0 \(MIT\)/);
});

test("a shipped crate without a license text fails the generation", () => {
  const packages = [
    { name: "alpha", version: "2.0.0", license: "MIT" },
    { name: "ratatui", version: "0.30.0", license: "MIT" },
  ];
  assert.throws(
    () => renderNotices(packages, (pkg) => (pkg.name === "alpha" ? ["MIT text\n"] : [])),
    /no license text for:\n {2}ratatui 0\.30\.0 \(MIT\)/,
  );
});
