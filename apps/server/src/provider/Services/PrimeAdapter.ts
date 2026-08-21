/**
 * PrimeAdapter — shape type for the Prime Agent adapter.
 *
 * The driver model ({@link ../Drivers/PrimeDriver}) bundles one adapter per
 * instance as a captured closure, so this module only retains the shape
 * interface as a naming anchor for the driver bundle.
 *
 * @module PrimeAdapter
 */
import type { ProviderAdapterError } from "../Errors.ts";
import type { ProviderAdapterShape } from "./ProviderAdapter.ts";

/**
 * PrimeAdapterShape — per-instance Prime Agent adapter contract.
 */
export interface PrimeAdapterShape extends ProviderAdapterShape<ProviderAdapterError> {}
