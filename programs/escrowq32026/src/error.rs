use anchor_lang::prelude::*;

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("The requested amount has changed")]
    OfferChanged,
}
