export function checkout(items: number, member: boolean) {
  let fee = 0
  if (items === 0) return 0
  if (member) fee -= 5
  return fee
}
export function total(a: number, b: number) {
  return a + b
}
