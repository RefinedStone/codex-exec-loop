// 입력창의 이미지 첨부이다 (#2838). 붙여넣거나 끌어 놓은 이미지를 data URL 로 바꿔 다음 메시지의 `image` 입력으로
// 보낸다. 첨부는 그 입력창(스레드)의 메모리에만 있고 저장하지 않는다. 앱을 다시 열거나 다른 스레드로 가면 없어진다.

export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
export const MAX_IMAGES = 4;

export interface ImageAttachment {
  id: string;
  name: string;
  /** `data:image/...;base64,...` 이다. */
  url: string;
}

export interface AttachmentFile {
  name: string;
  type: string;
  size: number;
}

/** 첨부할 수 없는 이유이다. 첨부할 수 있으면 null 이다. */
export function refuseAttachment(file: AttachmentFile, attachedCount: number): string | null {
  if (!file.type.startsWith("image/")) return `이미지 파일만 첨부할 수 있습니다: ${file.name}`;
  if (file.size > MAX_IMAGE_BYTES) return `이미지는 ${MAX_IMAGE_BYTES / 1024 / 1024}MB 까지 첨부할 수 있습니다: ${file.name}`;
  if (attachedCount >= MAX_IMAGES) return `이미지는 ${MAX_IMAGES}개까지 첨부할 수 있습니다.`;
  return null;
}

/** 보낸 첨부만 뺀다. 보내는 동안 새로 붙인 첨부는 남긴다. */
export function withoutSent(current: readonly ImageAttachment[], sent: readonly ImageAttachment[]): ImageAttachment[] {
  const sentIds = new Set(sent.map((image) => image.id));
  return current.filter((image) => !sentIds.has(image.id));
}

/** 파일을 data URL 로 읽는다. */
export function readImage(file: File): Promise<ImageAttachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`이미지를 읽지 못했습니다: ${file.name}`));
    reader.onload = () => resolve({ id: `${Date.now()}-${Math.random().toString(16).slice(2)}`, name: file.name || "붙여넣은 이미지", url: String(reader.result) });
    reader.readAsDataURL(file);
  });
}
