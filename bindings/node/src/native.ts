// Native napi surface with the package's canonical JavaScript Error classes.
// Rust constructs these classes directly, so native throws and promise
// rejections preserve their prototype chain.

import * as native from '../index.js'
import * as errors from './error.ts'

native.__registerErrorClasses(errors)

export * from '../index.js'
export * from './error.ts'
