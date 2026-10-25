import { v7, validate, version } from 'uuid'

export const newId = (): string => v7()
export const validUUID = (value: unknown): value is string =>
  typeof value === 'string' && validate(value) && version(value) === 7
