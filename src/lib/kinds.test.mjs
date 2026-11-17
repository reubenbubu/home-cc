// Run with "npm test" (Node reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { ADD_AS, isPrivateMac, looksHandheld, suggestedKind, usuallyNoSsh } from "./kinds.ts";

const REAL_MAC = "3c:52:a1:10:2f:01";
const MADE_UP_MAC = "9a:3f:c2:71:0b:e4";
const seen = (hostname, vendor = null, mac = REAL_MAC) => ({ hostname, vendor, mac });

test("appliances and handhelds are the types that usually have no SSH", () => {
  assert.ok(usuallyNoSsh("appliance"));
  assert.ok(usuallyNoSsh("handheld"));
  for (const kind of ["pc", "server", "vm", "container", "mesh", "other", "ignore"]) assert.ok(!usuallyNoSsh(kind), kind);
});

test("a found device can be added as a handheld, before Other", () => {
  assert.ok(ADD_AS.includes("handheld"));
  assert.ok(ADD_AS.indexOf("handheld") < ADD_AS.indexOf("other"));
  assert.ok(!ADD_AS.includes("ignore"), "Ignore has its own button");
});

test("a made-up hardware address is told from a maker's own", () => {
  assert.ok(isPrivateMac(MADE_UP_MAC));
  assert.ok(isPrivateMac("DA:A1:19:5C:E0:27"));
  assert.ok(!isPrivateMac(REAL_MAC));
  assert.ok(!isPrivateMac(null));
});

test("the names phones, tablets and handheld consoles give themselves are enough by themselves", () => {
  for (const name of [
    "iPhone",
    "Sams-iPhone",
    "iPad-Pro",
    "android-3f2a9b0c1d4e5f60",
    "Pixel-8",
    "Google-Pixel-7a",
    "Pixel-Tablet",
    "Galaxy-S24",
    "GalaxyS23",
    "Galaxy-Tab-S9",
    "SM-S921B",
    "SM-T870",
    "steamdeck",
    "Steam-Deck",
    "Nintendo-Switch",
    "Kindle-Paperwhite",
    "ROG-Ally",
  ]) {
    assert.ok(looksHandheld(seen(name)), name);
  }
});

test("a word that can mean something else counts only with a maker of handhelds or a made-up address", () => {
  // A network switch, a desk phone and a docking station are not handhelds.
  assert.ok(!looksHandheld(seen("switch-core")));
  assert.ok(!looksHandheld(seen("switch-core", "NETGEAR")));
  assert.ok(!looksHandheld(seen("VoIP-Phone", "Grandstream Networks, Inc.")));
  assert.ok(!looksHandheld(seen("tablet-dock")));
  // Backed up by the maker, or by a made-up address (as most phones have).
  assert.ok(looksHandheld(seen("Switch", "Nintendo Co.,Ltd")));
  assert.ok(looksHandheld(seen("Sams-Phone", "Apple, Inc.")));
  assert.ok(looksHandheld(seen("Sams-Phone", null, MADE_UP_MAC)));
  assert.ok(looksHandheld(seen("Kitchen-Tablet", "Lenovo Mobile Communication Technology Ltd.")));
  assert.ok(looksHandheld(seen("OnePlus-12", null, MADE_UP_MAC)));
  // "vivo" the maker, not a word inside another maker's name.
  assert.ok(!looksHandheld(seen("Garden-Phone", "VIVOTEK INC.")));
});

test("the maker alone is never enough: the same makers make TVs and computers", () => {
  assert.ok(!looksHandheld(seen("Samsung-TV", "Samsung Electronics Co.,Ltd")));
  assert.ok(!looksHandheld(seen("BRW1C2D3E4F5A6B", "Apple, Inc.")));
  assert.ok(!looksHandheld(seen(null, "Apple, Inc.", MADE_UP_MAC)));
  assert.ok(!looksHandheld(seen("  ", "Samsung Electronics Co.,Ltd", MADE_UP_MAC)));
  assert.ok(!looksHandheld(seen(null)));
});

test("a TV, a laptop, a watch or earbuds with a handheld's word in its name is not one", () => {
  for (const name of ["Android-TV", "Galaxy-Book3", "Pixelbook-Go", "Galaxy-Watch6", "Galaxy-Buds2", "Sams-MacBook-Pro", "Phone-Desktop"]) {
    assert.ok(!looksHandheld(seen(name, "Samsung Electronics Co.,Ltd", MADE_UP_MAC)), name);
  }
  // And a PC whose name has none of the words.
  assert.ok(!looksHandheld(seen("DESKTOP-MAIN", "ASUSTek Computer Inc.")));
  assert.ok(!looksHandheld(seen("pixelpc")));
});

test("a found device is offered what the scan set, or Handheld when its hostname says so, never more", () => {
  const found = (kind, hostname, vendor = null, mac = MADE_UP_MAC) => ({ kind, hostname, vendor, mac });
  assert.equal(suggestedKind(found("other", "Galaxy-S24")), "handheld");
  assert.equal(suggestedKind(found("other", null)), "other");
  assert.equal(suggestedKind(found("other", "Samsung-TV", "Samsung Electronics Co.,Ltd", REAL_MAC)), "other");
  // A mesh node stays one, whatever its name.
  assert.equal(suggestedKind(found("mesh", "deco-BE85")), "mesh");
  assert.equal(suggestedKind(found("mesh", "iPhone")), "mesh");
  // A type that has no button of its own is offered as Other, or Handheld by its name.
  assert.equal(suggestedKind(found("container", "pihole1")), "other");
  assert.equal(suggestedKind(found("ignore", "Pixel-8")), "handheld");
  assert.equal(suggestedKind(found("pc", "Pixel-8")), "pc");
});
