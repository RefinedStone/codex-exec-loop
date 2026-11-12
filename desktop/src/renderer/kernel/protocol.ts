// Wire types come from the Rust kernel spec. Only the panel's query selection lives here.
import { KERNEL_ERROR, type KernelQuery, type KernelQueryAnswer } from "../../shared/kernel-protocol/generated";

export * from "../../shared/kernel-protocol/generated";
export const KERNEL_UNAVAILABLE = KERNEL_ERROR.kernel_unavailable;

export const KERNEL_VIEW_QUERIES = ["thread_activity", "auto_follow", "planning", "parallel"] as const satisfies readonly KernelQuery["type"][];
export type KernelViewQuery = (typeof KERNEL_VIEW_QUERIES)[number];
export type KernelViewAnswer = Extract<KernelQueryAnswer, { type: KernelViewQuery }>;
