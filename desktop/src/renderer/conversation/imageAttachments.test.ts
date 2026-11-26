import { describe, expect, it } from "vitest";

import { MAX_IMAGE_BYTES, MAX_IMAGES, refuseAttachment } from "./imageAttachments";

describe("image attachments (#2838)", () => {
  it("accepts images within the limits and says why it refuses others", () => {
    expect(refuseAttachment({ name: "a.png", type: "image/png", size: 10 }, 0)).toBeNull();
    expect(refuseAttachment({ name: "a.txt", type: "text/plain", size: 10 }, 0)).toContain("이미지 파일만");
    expect(refuseAttachment({ name: "big.png", type: "image/png", size: MAX_IMAGE_BYTES + 1 }, 0)).toContain("MB 까지");
    expect(refuseAttachment({ name: "b.png", type: "image/png", size: 10 }, MAX_IMAGES)).toContain(`${MAX_IMAGES}개까지`);
  });
});
