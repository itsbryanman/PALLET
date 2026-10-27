# Recovery

At startup, PALLET reads `.pallet/journal.log` before accepting a new operation. For an interrupted move it compares the recorded device/inode identity at the original and destination locations:

- source only: records `NOT_MOVED`;
- destination only: records `RECOVERED_COMMITTED`;
- both, neither, an identity mismatch, a changed root, or corruption in the journal middle: freezes the warehouse.

PALLET does not delete a file to resolve ambiguity. A torn final journal record also freezes the warehouse until the user acknowledges the preserved evidence. Process-kill testing covers the commit boundaries; it is not a power-loss durability claim.
