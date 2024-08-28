import { describe, test } from 'node:test'
import * as assert from 'node:assert'

import { sendDataItemWith } from './sendDataItem.js'
import { Resolved } from 'hyper-async'

const logger = () => undefined
logger.tap = () => (args) => {
  return args
}

describe('sendDataItemWith', () => {
  describe('Send Data Item', () => {
    describe('Send process', () => {
      test('Send process without target for assignment', async () => {
        let cranked = false
        const sendDataItem = sendDataItemWith({
          selectNode: () => 'cu-url',
          createDataItem: (raw) => ({
            id: 'process-id',
            tags: [
              { name: 'Data-Protocol', value: 'ao' },
              { name: 'Type', value: 'Process' },
              { name: 'Scheduler', value: 'scheduler-id' }
            ]
          }),
          writeDataItem: async (res) => ({
            ...res,
            id: 'scheduler-id',
            timestamp: 1234567
          }),
          locateScheduler: async () => ({ url: 'url-123' }),
          locateProcess: async (res) => {
            return ({
              url: 'url-1234',
              address: 'address-123'
            })
          },
          fetchResult: (res) => res,
          crank: () => {
            cranked = true
            return Resolved()
          },
          logger,
          fetchSchedulerProcess: (res) => res,
          writeDataItemArweave: (res) => res
        })

        const { crank, ...result } = await sendDataItem({
          raw: '1234',
          tx: { id: 'process-id' }
        }).toPromise()

        assert.equal(result.schedulerTx.id, 'scheduler-id')
        assert.equal(result.schedulerTx.timestamp, 1234567)

        await crank().toPromise()
        assert.ok(!cranked)
      })

      test('Send process with target for assignment', async () => {
        let cranked = false
        const sendDataItem = sendDataItemWith({
          selectNode: () => 'cu-url',
          createDataItem: (raw) => ({
            id: 'process-id',
            target: 'target-process-id',
            tags: [
              { name: 'Data-Protocol', value: 'ao' },
              { name: 'Type', value: 'Process' },
              { name: 'Scheduler', value: 'scheduler-id' }
            ]
          }),
          writeDataItem: async (res) => ({
            ...res,
            id: 'scheduler-id',
            timestamp: 1234567
          }),
          locateScheduler: async () => ({ url: 'url-123' }),
          locateProcess: (res) => res,
          fetchResult: (res) => res,
          crank: ({ assigns, initialTxId }) => {
            cranked = true
            assert.deepStrictEqual(assigns, [{ Message: 'process-id', Processes: ['target-process-id'] }])
            assert.equal(initialTxId, 'process-id')
            return Resolved()
          },
          logger,
          fetchSchedulerProcess: (res) => res,
          writeDataItemArweave: (res) => res
        })

        const { crank, ...result } = await sendDataItem({
          raw: '1234',
          tx: { id: 'process-id' }
        }).toPromise()

        await crank().toPromise()

        assert.ok(cranked)
        assert.equal(result.schedulerTx.id, 'scheduler-id')
        assert.equal(result.schedulerTx.timestamp, 1234567)
      })
    })
  })
})
