export function shippingFee(weight: number, express: boolean) {
  let cost = weight > 20 ? 18 : 6
  if (express) cost *= 2
  return cost
}

export function orderTotal(subtotal: number, fee: number) {
  return subtotal + fee
}
