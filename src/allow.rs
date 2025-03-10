// This file contains attributes to allow certain Clippy lints project-wide.
// It's easier to allow these lints in one place than to add annotations to each instance
// throughout the codebase. Over time, these can be fixed properly.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::cognitive_complexity)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::stable_sort_primitive)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::doc_markdown)] 
#![allow(clippy::option_if_let_else)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::branches_sharing_code)] 